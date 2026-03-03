#!/usr/bin/env python3
"""
Live display test - Captures, encodes, decodes and displays video with FPS counter.

Requirements:
    pip install opencv-python numpy pillow av
"""
import asyncio
import sys
import time
import threading
from pathlib import Path

# Add src to path
sys.path.insert(0, str(Path(__file__).parent / 'src'))

import cv2
import numpy as np


class LiveDisplayTest:
    """Live capture-encode-decode-display test with FPS counter."""

    def __init__(
        self,
        width: int = 1280,
        height: int = 720,
        target_fps: int = 30,
        backend: str = "pil"
    ):
        self.width = width
        self.height = height
        self.target_fps = target_fps
        self.backend = backend

        # Statistics
        self.frame_count = 0
        self.encode_count = 0
        self.decode_count = 0
        self.start_time = 0
        self.last_fps_update = 0
        self.current_fps = 0

        # Threading
        self.running = False
        self.display_queue = None

        # Components
        self.capturer = None
        self.encoder = None
        self.decoder = None

    async def initialize(self):
        """Initialize all components."""
        print("Initializing components...")

        # Import here for better error messages
        try:
            from capture.d3dshot_backend import ScreenCapturer
        except ImportError as e:
            print(f"❌ Failed to import capturer: {e}")
            return False

        try:
            import av
        except ImportError:
            print("❌ PyAV not installed. Run: pip install av")
            return False

        # Initialize capturer
        self.capturer = ScreenCapturer(
            target_fps=self.target_fps,
            preferred_backend=self.backend
        )

        if not await self.capturer.initialize():
            print("❌ Capturer initialization failed")
            return False

        print(f"   ✅ Capturer: {self.capturer.screen_width}x{self.capturer.screen_height}")

        # Adjust resolution to capturer's actual resolution
        self.width = self.capturer.screen_width
        self.height = self.capturer.screen_height

        # Initialize encoder (simple container-based)
        import io
        self.encoder_output = io.BytesIO()
        self.encoder_container = av.open(self.encoder_output, 'w', format='h264')
        self.encoder_stream = self.encoder_container.add_stream('libx264', rate=self.target_fps)
        self.encoder_stream.width = self.width
        self.encoder_stream.height = self.height
        self.encoder_stream.bit_rate = 3_000_000
        self.encoder_pts = 0

        print(f"   ✅ Encoder: libx264 @ {self.target_fps}fps")

        # Initialize decoder
        self.decoder_input = io.BytesIO()
        self.decoder_frames = []

        print(f"   ✅ All components initialized")
        return True

    def run_display_thread(self):
        """Display thread - shows decoded frames with FPS overlay."""
        cv2.namedWindow("Live Display Test", cv2.WINDOW_NORMAL)
        cv2.setWindowProperty("Live Display Test", cv2.WND_PROP_FULLSCREEN, cv2.WINDOW_FULLSCREEN)

        while self.running:
            if self.display_queue and len(self.display_queue) > 0:
                frame_data = self.display_queue.pop(0)

                # Convert to BGR for OpenCV
                frame = cv2.cvtColor(frame_data, cv2.COLOR_RGB2BGR)

                # Add FPS overlay
                fps_text = f"FPS: {self.current_fps:.1f}"
                stats_text = f"Frames: {self.frame_count} | Encoded: {self.encode_count} | Decoded: {self.decode_count}"
                resolution_text = f"Resolution: {self.width}x{self.height}"

                # Draw semi-transparent background
                overlay = frame.copy()
                cv2.rectangle(overlay, (10, 10), (400, 100), (0, 0, 0), -1)
                frame = cv2.addWeighted(overlay, 0.6, frame, 0.4, 0)

                # Draw text
                cv2.putText(frame, fps_text, (20, 40), cv2.FONT_HERSHEY_SIMPLEX,
                           1.0, (0, 255, 0), 2)
                cv2.putText(frame, stats_text, (20, 65), cv2.FONT_HERSHEY_SIMPLEX,
                           0.5, (255, 255, 255), 1)
                cv2.putText(frame, resolution_text, (20, 85), cv2.FONT_HERSHEY_SIMPLEX,
                           0.5, (255, 255, 255), 1)

                # Show frame
                cv2.imshow("Live Display Test", frame)

            # Exit on ESC
            if cv2.waitKey(1) & 0xFF == 27:
                self.running = False
                break

        cv2.destroyAllWindows()

    async def run(self, duration: int = 60):
        """Run the live test."""
        print(f"\n{'='*60}")
        print(f"Live Display Test - {duration} seconds")
        print(f"{'='*60}")
        print(f"Press ESC in window to exit early")
        print(f"{'='*60}\n")

        # Initialize
        if not await self.initialize():
            return

        # Start display thread
        self.display_queue = []
        self.running = True
        self.start_time = time.time()
        self.last_fps_update = time.time()

        display_thread = threading.Thread(target=self.run_display_thread, daemon=True)
        display_thread.start()

        import av

        # Main loop
        end_time = time.time() + duration
        last_encode_flush = time.time()

        while time.time() < end_time and self.running:
            loop_start = time.time()

            # Capture
            captured = await self.capturer.capture_frame()
            if captured is None:
                continue

            self.frame_count += 1

            # Encode (in thread pool to avoid blocking)
            try:
                import numpy as np
                arr = np.frombuffer(captured.data, dtype=np.uint8)
                arr = arr.reshape((captured.height, captured.width, 3))
                frame = av.VideoFrame.from_ndarray(arr, format='rgb24')
                frame.pts = self.encoder_pts
                self.encoder_pts += 1

                # Encode
                for packet in self.encoder_stream.encode(frame):
                    self.encoder_container.mux(packet)
                    self.encode_count += 1

                # Periodically flush encoder to get output
                if time.time() - last_encode_flush > 0.5:
                    for packet in self.encoder_stream.encode():
                        self.encoder_container.mux(packet)

                    # Get encoded data and decode
                    encoded_size = self.encoder_output.tell()
                    if encoded_size > 1000:
                        self.encoder_output.seek(0)
                        encoded_data = self.encoder_output.read()

                        # Decode
                        input_buffer = io.BytesIO(encoded_data)
                        try:
                            input_container = av.open(input_buffer, 'r', format='h264')
                            for packet in input_container.demux():
                                for decoded_frame in packet.decode():
                                    if decoded_frame.width > 0:
                                        # Convert to RGB
                                        rgb_frame = decoded_frame.to_ndarray(format='rgb24')

                                        # Add to display queue
                                        if len(self.display_queue) < 3:
                                            self.display_queue.append(rgb_frame)
                                        self.decode_count += 1
                                        break
                                break
                        except Exception:
                            pass  # Decoder might not have complete data yet

                        # Reset output buffer
                        self.encoder_output = io.BytesIO()
                        self.encoder_container = av.open(self.encoder_output, 'w', format='h264')
                        self.encoder_stream = self.encoder_container.add_stream('libx264', rate=self.target_fps)
                        self.encoder_stream.width = self.width
                        self.encoder_stream.height = self.height
                        self.encoder_stream.bit_rate = 3_000_000

                    last_encode_flush = time.time()

            except Exception as e:
                print(f"Encode/Decode error: {e}")

            # Update FPS
            now = time.time()
            if now - self.last_fps_update >= 0.5:
                elapsed = now - self.last_fps_update
                frames = self.frame_count
                # Approximate FPS since last update
                self.current_fps = frames / (now - self.start_time)
                self.last_fps_update = now

            # Frame pacing
            elapsed = time.time() - loop_start
            target_time = 1.0 / self.target_fps
            if elapsed < target_time:
                await asyncio.sleep(target_time - elapsed)

        # Cleanup
        print("\n" + "="*60)
        print("Test Complete")
        print("="*60)
        elapsed_total = time.time() - self.start_time
        print(f"Duration: {elapsed_total:.1f}s")
        print(f"Frames captured: {self.frame_count}")
        print(f"Frames encoded: {self.encode_count}")
        print(f"Frames decoded: {self.decode_count}")
        print(f"Average FPS: {self.frame_count / elapsed_total:.1f}")


async def main():
    """Run the live display test."""
    import argparse

    parser = argparse.ArgumentParser(description="Live display test")
    parser.add_argument("--duration", type=int, default=60, help="Test duration in seconds")
    parser.add_argument("--fps", type=int, default=30, help="Target FPS")
    parser.add_argument("--backend", type=str, default="pil", choices=["pil", "gdi", "mss"], help="Capture backend")
    args = parser.parse_args()

    test = LiveDisplayTest(
        width=1280,
        height=720,
        target_fps=args.fps,
        backend=args.backend
    )

    try:
        await test.run(duration=args.duration)
    except KeyboardInterrupt:
        print("\n\nTest interrupted by user")
    finally:
        test.running = False


if __name__ == "__main__":
    # Check for opencv
    try:
        import cv2
    except ImportError:
        print("❌ OpenCV not installed!")
        print("   Run: pip install opencv-python")
        sys.exit(1)

    asyncio.run(main())
