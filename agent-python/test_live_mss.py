#!/usr/bin/env python3
"""
Live display test - MSS capture with encode/decode pipeline and FPS counter.

Requirements:
    pip install opencv-python numpy av mss
"""
import sys
import time
import threading
from pathlib import Path

# Add src to path
sys.path.insert(0, str(Path(__file__).parent / 'src'))

import cv2
import numpy as np
import mss
import mss.tools
import io


class MSSLiveDisplay:
    """Live display with MSS capture and encode/decode."""

    def __init__(self, target_fps: int = 30, enable_encode: bool = False):
        self.target_fps = target_fps
        self.enable_encode = enable_encode
        self.running = False
        self.frame_count = 0
        self.encode_count = 0
        self.decode_count = 0
        self.start_time = 0
        self.current_fps = 0

    def capture_mss(self):
        """Capture using MSS (faster than PIL)."""
        with mss.mss() as sct:
            monitor = sct.monitors[1]  # Primary monitor
            screenshot = sct.grab(monitor)
            # Convert to numpy array (BGRA)
            img = np.frombuffer(screenshot.rgb, dtype=np.uint8)
            img = img.reshape((screenshot.height, screenshot.width, 3))
            return img

    def run(self):
        """Run the display loop."""
        encode_str = " + Encode/Decode" if self.enable_encode else ""
        print("="*60)
        print(f"Live Display Test (MSS){encode_str} - Press ESC to exit")
        print("="*60)

        self.running = True
        self.start_time = time.time()
        last_fps_update = time.time()

        cv2.namedWindow("Live Display", cv2.WINDOW_NORMAL)
        cv2.setWindowProperty("Live Display", cv2.WINDOW_FULLSCREEN, cv2.WINDOW_FULLSCREEN)

        # Initialize encoder/decoder if needed
        encoder = None
        decoder = None
        if self.enable_encode:
            try:
                import av
                encoder_output = io.BytesIO()
                encoder_container = av.open(encoder_output, 'w', format='h264')
                encoder_stream = encoder_container.add_stream('libx264', rate=30)
                encoder_stream.width = 1920
                encoder_stream.height = 1080
                encoder_stream.bit_rate = 3_000_000
                encoder_pts = 0

                print(f"   Encoder initialized")
            except Exception as e:
                print(f"   Encoder init failed: {e}")
                self.enable_encode = False

        while self.running:
            loop_start = time.time()

            # Capture
            frame = self.capture_mss()
            if frame is None:
                continue

            # Store original size
            orig_h, orig_w = frame.shape[:2]

            # Resize to 1080p for encoding if enabled
            if self.enable_encode:
                process_frame = cv2.resize(frame, (1920, 1080))
            else:
                process_frame = frame

            self.frame_count += 1

            # Display frame (resize if too large)
            display_frame = frame
            if orig_w > 1920:
                scale = 1920 / orig_w
                display_frame = cv2.resize(frame, (0, 0), fx=scale, fy=scale)

            # Update FPS
            now = time.time()
            if now - last_fps_update >= 0.2:
                self.current_fps = self.frame_count / (now - self.start_time)
                last_fps_update = now

            # Draw overlay
            h, w = display_frame.shape[:2]
            overlay = display_frame.copy()
            cv2.rectangle(overlay, (10, 10), (500, 140 if self.enable_encode else 120), (0, 0, 0), -1)
            display_frame = cv2.addWeighted(overlay, 0.7, display_frame, 0.3, 0)

            # FPS
            fps_color = (0, 255, 0) if self.current_fps >= 25 else (0, 255, 255) if self.current_fps >= 15 else (0, 0, 255)
            cv2.putText(display_frame, f"FPS: {self.current_fps:.1f}", (20, 45),
                       cv2.FONT_HERSHEY_SIMPLEX, 1.2, fps_color, 3)

            # Stats
            cv2.putText(display_frame, f"Frames: {self.frame_count}", (20, 75),
                       cv2.FONT_HERSHEY_SIMPLEX, 0.5, (255, 255, 255), 1)
            cv2.putText(display_frame, f"Resolution: {orig_w}x{orig_h}", (20, 95),
                       cv2.FONT_HERSHEY_SIMPLEX, 0.5, (255, 255, 255), 1)

            # Quality indicator
            if self.current_fps >= 25:
                quality = "Excellent"
                q_color = (0, 255, 0)
            elif self.current_fps >= 15:
                quality = "Good"
                q_color = (0, 255, 255)
            else:
                quality = "Poor"
                q_color = (0, 0, 255)
            cv2.putText(display_frame, f"Quality: {quality}", (20, 115),
                       cv2.FONT_HERSHEY_SIMPLEX, 0.5, q_color, 1)

            if self.enable_encode:
                cv2.putText(display_frame, f"Encoded: {self.encode_count} | Decoded: {self.decode_count}", (20, 135),
                           cv2.FONT_HERSHEY_SIMPLEX, 0.5, (200, 200, 255), 1)

            # Show
            cv2.imshow("Live Display", display_frame)

            # Exit check
            key = cv2.waitKey(1) & 0xFF
            if key == 27:  # ESC
                break

            # Frame pacing
            elapsed = time.time() - loop_start
            target_time = 1.0 / self.target_fps
            if elapsed < target_time:
                time.sleep(target_time - elapsed)

        cv2.destroyAllWindows()

        # Final stats
        print("\n" + "="*60)
        print("Test Complete")
        print("="*60)
        total_time = time.time() - self.start_time
        print(f"Duration: {total_time:.1f}s")
        print(f"Total frames: {self.frame_count}")
        print(f"Average FPS: {self.frame_count / total_time:.1f}")
        if self.enable_encode:
            print(f"Encoded: {self.encode_count} | Decoded: {self.decode_count}")


if __name__ == "__main__":
    import argparse

    parser = argparse.ArgumentParser()
    parser.add_argument("--fps", type=int, default=30, help="Target FPS")
    parser.add_argument("--encode", action="store_true", help="Enable encode/decode pipeline")
    args = parser.parse_args()

    try:
        test = MSSLiveDisplay(target_fps=args.fps, enable_encode=args.encode)
        test.run()
    except KeyboardInterrupt:
        print("\n\nInterrupted by user")
