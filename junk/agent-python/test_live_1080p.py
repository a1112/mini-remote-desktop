#!/usr/bin/env python3
"""
Live display test - 1080p downscale for optimal performance.

Based on our benchmarks, 1080p achieves 60 FPS while 1440p only gets 30 FPS.
"""
import sys
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent / 'src'))

import cv2
import numpy as np
import mss


class LiveDisplay1080p:
    """Live display with 1080p downscale for optimal FPS."""

    def __init__(self, target_width=1920, target_height=1080):
        self.target_width = target_width
        self.target_height = target_height
        self.running = False

        # Stats
        self.frame_count = 0
        self.start_time = 0
        self.current_fps = 0

        # Initialize MSS
        self.sct = mss.mss()
        # Get primary monitor info
        self.monitor = self.sct.monitors[1]
        src_w = self.monitor["width"]
        src_h = self.monitor["height"]

        # Calculate scale
        scale_w = target_width / src_w
        scale_h = target_height / src_h
        self.scale = min(scale_w, scale_h)

        # Target size
        self.capture_w = int(src_w * self.scale)
        self.capture_h = int(src_h * self.scale)

        print(f"Source resolution: {src_w}x{src_h}")
        print(f"Target resolution: {self.capture_w}x{self.capture_h} (scaled)")

    def capture(self):
        """Capture and downscale."""
        # Capture at target resolution (MSS can downscale directly)
        monitor_region = {
            "left": 0,
            "top": 0,
            "width": self.capture_w,
            "height": self.capture_h,
            "mon": 1
        }

        screenshot = self.sct.grab(monitor_region)

        # Convert to numpy
        img = np.frombuffer(screenshot.rgb, dtype=np.uint8)
        img = img.reshape((screenshot.height, screenshot.width, 3))

        # Resize to target resolution
        if self.scale < 1.0:
            img = cv2.resize(img, (self.capture_w, self.capture_h),
                           interpolation=cv2.INTER_AREA)

        return img

    def run(self):
        """Run display loop."""
        print("="*60)
        print("Live Display Test - 1080p Downscale")
        print("="*60)
        print("Press ESC or Q to exit")
        print("="*60)

        self.running = True
        self.start_time = time.time()
        last_fps_update = self.start_time

        cv2.namedWindow("Live Capture (1080p)", cv2.WINDOW_NORMAL)

        while self.running:
            loop_start = time.perf_counter()

            # Capture
            frame = self.capture()
            if frame is None:
                continue

            self.frame_count += 1

            # Update FPS
            now = time.time()
            if now - last_fps_update >= 0.2:
                self.current_fps = self.frame_count / (now - self.start_time)
                last_fps_update = now

            # Draw overlay
            h, w = frame.shape[:2]

            # Background
            overlay = frame.copy()
            cv2.rectangle(overlay, (5, 5), (380, 140), (0, 0, 0), -1)
            frame = cv2.addWeighted(overlay, 0.6, frame, 0.4, 0)

            # FPS with color
            if self.current_fps >= 50:
                fps_color = (0, 200, 0)
                rating = "⭐ Excellent"
            elif self.current_fps >= 30:
                fps_color = (0, 200, 200)
                rating = "⭐⭐ Good"
            elif self.current_fps >= 20:
                fps_color = (0, 150, 255)
                rating = "⭐ Fair"
            else:
                fps_color = (0, 0, 255)
                rating = "❌ Poor"

            cv2.putText(frame, f"FPS: {self.current_fps:.1f}", (15, 40),
                       cv2.FONT_HERSHEY_SIMPLEX, 1.0, fps_color, 2)

            cv2.putText(frame, f"Frames: {self.frame_count}", (15, 70),
                       cv2.FONT_HERSHEY_SIMPLEX, 0.45, (255, 255, 255), 1)
            cv2.putText(frame, f"Resolution: {w}x{h}", (15, 90),
                       cv2.FONT_HERSHEY_SIMPLEX, 0.45, (255, 255, 255), 1)
            cv2.putText(frame, f"Mode: 1080p Downscale", (15, 110),
                       cv2.FONT_HERSHEY_SIMPLEX, 0.45, (200, 200, 255), 1)
            cv2.putText(frame, rating, (15, 135),
                       cv2.FONT_HERSHEY_SIMPLEX, 0.5, fps_color, 1)

            # Show
            cv2.imshow("Live Capture (1080p)", frame)

            # Exit
            key = cv2.waitKey(1) & 0xFF
            if key == 27 or key == ord('q'):
                break

            # Frame pacing
            frame_time = time.perf_counter() - loop_start
            min_time = 1.0 / 60
            if frame_time < min_time:
                time.sleep(min_time - frame_time)

        cv2.destroyAllWindows()

        # Final stats
        total_time = time.time() - self.start_time

        print("\n" + "="*60)
        print("Test Complete")
        print("="*60)
        print(f"Duration: {total_time:.1f}s")
        print(f"Frames: {self.frame_count}")
        print(f"Average FPS: {self.frame_count / total_time:.1f}")
        print(f"Resolution: {self.capture_w}x{self.capture_h}")
        print("="*60)


if __name__ == "__main__":
    try:
        test = LiveDisplay1080p(target_width=1920, target_height=1080)
        test.run()
    except KeyboardInterrupt:
        print("\n\nInterrupted")
