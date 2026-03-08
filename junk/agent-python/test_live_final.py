#!/usr/bin/env python3
"""
Live display test - High performance capture with FPS counter.

Run until user presses ESC.
"""
import sys
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent / 'src'))

import cv2
import numpy as np

# Try to import mss for fast capture
try:
    import mss
    HAS_MSS = True
except ImportError:
    HAS_MSS = False
    print("Warning: mss not available, using PIL")

try:
    from PIL import ImageGrab
    HAS_PIL = True
except ImportError:
    HAS_PIL = False


class LiveDisplay:
    """High performance live display."""

    def __init__(self, backend="auto"):
        self.backend = backend
        self.running = False

        # Performance tracking
        self.frame_times = []
        self.fps_update_interval = 0.2
        self.last_fps_update = 0
        self.current_fps = 0
        self.frame_count = 0

        # Initialize capture
        if HAS_MSS and backend in ("auto", "mss"):
            self.sct = mss.mss()
            self.monitor = self.sct.monitors[1]
            self.capture_func = self._capture_mss
            print(f"Using MSS capture backend")
        elif HAS_PIL and backend in ("auto", "pil"):
            self.capture_func = self._capture_pil
            print(f"Using PIL capture backend")
        else:
            raise RuntimeError("No capture backend available!")

    def _capture_mss(self):
        """Capture with MSS."""
        screenshot = self.sct.grab(self.monitor)
        img = np.frombuffer(screenshot.rgb, dtype=np.uint8)
        img = img.reshape((screenshot.height, screenshot.width, 3))
        return img

    def _capture_pil(self):
        """Capture with PIL."""
        img = ImageGrab.grab()
        return cv2.cvtColor(np.array(img), cv2.COLOR_RGB2BGR)

    def run(self):
        """Run display loop."""
        print("="*60)
        print("Live Display Test")
        print("="*60)
        print("Press ESC or Q to exit")
        print("Close the window to stop")
        print("="*60)

        self.running = True
        start_time = time.time()
        self.last_fps_update = start_time

        # Create window
        cv2.namedWindow("Agent Live Capture", cv2.WINDOW_NORMAL)

        while self.running:
            loop_start = time.perf_counter()

            # Capture
            frame = self.capture_func()
            if frame is None:
                continue

            # Stats
            self.frame_count += 1
            now = time.time()

            # Update FPS
            if now - self.last_fps_update >= self.fps_update_interval:
                self.current_fps = self.frame_count / (now - start_time)
                self.last_fps_update = now

            # Draw overlay
            h, w = frame.shape[:2]

            # Scale down for display if needed
            scale = 1.0
            if w > 1920:
                scale = 1920 / w

            if scale < 1.0:
                display_w = int(w * scale)
                display_h = int(h * scale)
                display_frame = cv2.resize(frame, (display_w, display_h))
            else:
                display_frame = frame

            # Dark background for text
            dh, dw = display_frame.shape[:2]
            overlay = display_frame.copy()
            cv2.rectangle(overlay, (5, 5), (350, 130), (0, 0, 0), -1)
            cv2.addWeighted(overlay, 0.6, display_frame, 0.4, 0, display_frame)

            # FPS - colored by performance
            if self.current_fps >= 25:
                fps_color = (0, 200, 0)  # Green
                rating = "Excellent"
            elif self.current_fps >= 20:
                fps_color = (0, 200, 200)  # Cyan
                rating = "Good"
            elif self.current_fps >= 15:
                fps_color = (0, 150, 255)  # Yellow
                rating = "Fair"
            else:
                fps_color = (0, 0, 255)  # Red
                rating = "Poor"

            cv2.putText(display_frame, f"FPS: {self.current_fps:.1f}", (15, 40),
                       cv2.FONT_HERSHEY_SIMPLEX, 1.0, fps_color, 2)

            # Stats
            cv2.putText(display_frame, f"Frames: {self.frame_count}", (15, 70),
                       cv2.FONT_HERSHEY_SIMPLEX, 0.45, (255, 255, 255), 1)
            cv2.putText(display_frame, f"Resolution: {w}x{h}", (15, 90),
                       cv2.FONT_HERSHEY_SIMPLEX, 0.45, (255, 255, 255), 1)
            cv2.putText(display_frame, f"Backend: {self.backend.upper()}", (15, 110),
                       cv2.FONT_HERSHEY_SIMPLEX, 0.45, (200, 200, 200), 1)
            cv2.putText(display_frame, f"Rating: {rating}", (15, 130),
                       cv2.FONT_HERSHEY_SIMPLEX, 0.45, fps_color, 1)

            # Show
            cv2.imshow("Agent Live Capture", display_frame)

            # Input
            key = cv2.waitKey(1) & 0xFF
            if key == 27 or key == ord('q'):  # ESC or Q
                break

            # Simple frame pacing
            frame_time = time.perf_counter() - loop_start
            min_frame_time = 1.0 / 60  # Cap at 60 FPS
            if frame_time < min_frame_time:
                time.sleep(min_frame_time - frame_time)

        cv2.destroyAllWindows()

        # Final stats
        total_time = time.time() - start_time
        avg_fps = self.frame_count / total_time

        print("\n" + "="*60)
        print("Test Complete")
        print("="*60)
        print(f"Duration: {total_time:.1f}s")
        print(f"Frames captured: {self.frame_count}")
        print(f"Average FPS: {avg_fps:.1f}")
        print(f"Final FPS: {self.current_fps:.1f}")
        print(f"Backend: {self.backend}")
        print("="*60)


if __name__ == "__main__":
    import argparse

    parser = argparse.ArgumentParser(description="Live capture display test")
    parser.add_argument("--backend", choices=["auto", "mss", "pil"], default="auto",
                       help="Capture backend (default: auto)")
    args = parser.parse_args()

    try:
        display = LiveDisplay(backend=args.backend)
        display.run()
    except KeyboardInterrupt:
        print("\n\nInterrupted by user")
    except Exception as e:
        print(f"\nError: {e}")
        import traceback
        traceback.print_exc()
