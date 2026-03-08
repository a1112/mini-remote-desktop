#!/usr/bin/env python3
"""
Live display test - Simple capture and display with FPS counter.

Requirements:
    pip install opencv-python numpy pillow
"""
import sys
import time
import threading
from pathlib import Path

# Add src to path
sys.path.insert(0, str(Path(__file__).parent / 'src'))

import cv2
import numpy as np
from PIL import Image, ImageGrab


class SimpleLiveDisplay:
    """Simple live display with FPS counter."""

    def __init__(self, target_fps: int = 30):
        self.target_fps = target_fps
        self.running = False
        self.frame_count = 0
        self.start_time = 0
        self.current_fps = 0

    def capture_pil(self):
        """Capture using PIL.ImageGrab (most compatible)."""
        img = ImageGrab.grab()
        return cv2.cvtColor(np.array(img), cv2.COLOR_RGB2BGR)

    def run(self):
        """Run the display loop."""
        print("="*60)
        print("Live Display Test - Press ESC to exit")
        print("="*60)

        self.running = True
        self.start_time = time.time()
        last_fps_update = time.time()

        cv2.namedWindow("Live Display", cv2.WINDOW_NORMAL)
        cv2.setWindowProperty("Live Display", cv2.WND_PROP_FULLSCREEN, cv2.WINDOW_FULLSCREEN)

        while self.running:
            loop_start = time.time()

            # Capture
            frame = self.capture_pil()
            if frame is None:
                continue

            self.frame_count += 1

            # Resize for display (optional - keeps window manageable)
            display_frame = frame
            if frame.shape[1] > 1920:
                scale = 1920 / frame.shape[1]
                display_frame = cv2.resize(frame, (0, 0), fx=scale, fy=scale)

            # Add FPS overlay
            now = time.time()
            if now - last_fps_update >= 0.3:
                self.current_fps = self.frame_count / (now - self.start_time)
                last_fps_update = now

            # Draw overlay
            h, w = display_frame.shape[:2]

            # Semi-transparent background
            overlay = display_frame.copy()
            cv2.rectangle(overlay, (10, 10), (450, 120), (0, 0, 0), -1)
            display_frame = cv2.addWeighted(overlay, 0.7, display_frame, 0.3, 0)

            # FPS text
            fps_color = (0, 255, 0) if self.current_fps >= 25 else (0, 255, 255) if self.current_fps >= 15 else (0, 0, 255)
            cv2.putText(display_frame, f"FPS: {self.current_fps:.1f}", (20, 45),
                       cv2.FONT_HERSHEY_SIMPLEX, 1.2, fps_color, 3)

            # Stats
            cv2.putText(display_frame, f"Frames: {self.frame_count}", (20, 75),
                       cv2.FONT_HERSHEY_SIMPLEX, 0.5, (255, 255, 255), 1)
            cv2.putText(display_frame, f"Resolution: {frame.shape[1]}x{frame.shape[0]}", (20, 95),
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
        print(f"Final FPS: {self.current_fps:.1f}")


if __name__ == "__main__":
    import argparse

    parser = argparse.ArgumentParser()
    parser.add_argument("--fps", type=int, default=30, help="Target FPS")
    args = parser.parse_args()

    try:
        test = SimpleLiveDisplay(target_fps=args.fps)
        test.run()
    except KeyboardInterrupt:
        print("\n\nInterrupted by user")
