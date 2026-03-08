"""
Video display widget for remote desktop streams.

Uses software rendering via QImage for maximum compatibility.
"""

import logging
from typing import Optional

from PySide6.QtCore import Qt, QObject, Signal, Slot
from PySide6.QtGui import QImage, QPainter, QPen, QColor
from PySide6.QtWidgets import QWidget, QSizePolicy

import numpy as np
import numpy.typing as npt

logger = logging.getLogger(__name__)


class FrameSignal(QObject):
    """Signal for thread-safe frame updates."""
    frame_ready = Signal(object)  # Use object for numpy array


class VideoView(QWidget):
    """
    Video display widget using software rendering.

    Features:
    - Software rendering (QImage) for compatibility
    - Aspect ratio preservation
    - Scale mode selection (fit, fill, stretch)
    """

    def __init__(self, parent=None):
        """Initialize video view."""
        super().__init__(parent)

        self._frame: Optional[npt.NDArray[np.uint8]] = None
        self._scale_mode = "fit"
        self._maintain_aspect_ratio = True
        self._background_color = QColor(20, 20, 20)

        # Frame signal for thread-safe updates
        self._frame_signal = FrameSignal()
        self._frame_signal.frame_ready.connect(self._on_frame_received)

        # Setup UI
        self._setup_ui()

    def _setup_ui(self) -> None:
        """Setup UI components."""
        self.setSizePolicy(QSizePolicy.Expanding, QSizePolicy.Expanding)
        self.setMinimumSize(320, 240)
        self.setStyleSheet(f"background-color: {self._background_color.name()};")

    @Slot(object)
    def _on_frame_received(self, frame: npt.NDArray[np.uint8]) -> None:
        """Handle frame from signal."""
        self._frame = frame
        self.update()  # Trigger paintEvent

    def set_frame(self, frame: npt.NDArray[np.uint8]) -> None:
        """
        Set current video frame (thread-safe).

        Args:
            frame: RGB/RGBA numpy array (height, width, channels)
        """
        self._frame_signal.frame_ready.emit(frame)

    def clear_frame(self) -> None:
        """Clear current frame."""
        self._frame = None
        self.update()

    def set_scale_mode(self, mode: str) -> None:
        """Set scale mode: 'fit', 'fill', or 'stretch'."""
        if mode in ("fit", "fill", "stretch"):
            self._scale_mode = mode
            self.update()

    def set_maintain_aspect_ratio(self, maintain: bool) -> None:
        """Set whether to maintain aspect ratio."""
        self._maintain_aspect_ratio = maintain
        self.update()

    def paintEvent(self, event) -> None:
        """Paint the video frame."""
        super().paintEvent(event)

        painter = QPainter(self)
        painter.setRenderHint(QPainter.Antialiasing)
        painter.setRenderHint(QPainter.SmoothPixmapTransform)

        # Fill background
        painter.fillRect(self.rect(), self._background_color)

        if self._frame is None:
            self._paint_placeholder(painter)
            return

        # Convert numpy array to QImage
        height, width = self._frame.shape[:2]
        channels = self._frame.shape[2] if len(self._frame.shape) == 3 else 1

        # Determine format
        if channels == 3:
            format = QImage.Format.Format_RGB888
        elif channels == 4:
            format = QImage.Format.Format_RGBA8888
        elif channels == 1:
            format = QImage.Format.Format_Grayscale8
        else:
            logger.warning(f"Unsupported channel count: {channels}")
            self._paint_placeholder(painter)
            return

        # Create QImage from numpy array
        image = QImage(
            self._frame.data,
            width,
            height,
            self._frame.strides[0] if hasattr(self._frame, 'strides') else width * channels,
            format
        )

        if image.isNull():
            logger.error("Failed to create QImage from frame")
            self._paint_placeholder(painter)
            return

        # Calculate display rect
        dest_rect = self._calculate_display_rect(
            image.width(),
            image.height(),
            self.width(),
            self.height()
        )

        # Draw image
        painter.drawImage(dest_rect, image)

    def _paint_placeholder(self, painter: QPainter) -> None:
        """Paint placeholder when no frame is available."""
        rect = self.rect()

        # Draw border
        pen = QPen(QColor(60, 60, 60), 2)
        painter.setPen(pen)
        painter.drawRect(rect.adjusted(1, 1, -1, -1))

        # Draw text
        painter.setPen(QColor(120, 120, 120))
        font = painter.font()
        font.setPointSize(14)
        painter.setFont(font)

        text = "No Connection"
        metrics = painter.fontMetrics()
        text_rect = metrics.boundingRect(text)
        text_rect.moveCenter(rect.center())

        painter.drawText(text_rect, Qt.AlignmentFlag.AlignCenter, text)

    def _calculate_display_rect(
        self,
        frame_width: int,
        frame_height: int,
        view_width: int,
        view_height: int
    ):
        """Calculate destination rectangle for frame display."""
        from PySide6.QtCore import QRect

        if self._scale_mode == "stretch" or not self._maintain_aspect_ratio:
            return QRect(0, 0, view_width, view_height)

        # Calculate aspect ratio
        frame_aspect = frame_width / frame_height if frame_height > 0 else 1.0
        view_aspect = view_width / view_height if view_height > 0 else 1.0

        if self._scale_mode == "fit":
            # Fit within view, maintaining aspect ratio
            if view_aspect > frame_aspect:
                display_height = view_height
                display_width = int(view_height * frame_aspect)
                x = (view_width - display_width) // 2
                y = 0
            else:
                display_width = view_width
                display_height = int(view_width / frame_aspect)
                x = 0
                y = (view_height - display_height) // 2
        else:  # fill
            # Fill view, maintaining aspect ratio (crop if needed)
            if view_aspect > frame_aspect:
                display_width = view_width
                display_height = int(view_width / frame_aspect)
                x = 0
                y = (view_height - display_height) // 2
            else:
                display_height = view_height
                display_width = int(view_height * frame_aspect)
                x = (view_width - display_width) // 2
                y = 0

        return QRect(x, y, display_width, display_height)

    @property
    def frame(self) -> Optional[npt.NDArray[np.uint8]]:
        """Get current frame."""
        return self._frame

    @property
    def scale_mode(self) -> str:
        """Get current scale mode."""
        return self._scale_mode

    def cleanup(self) -> None:
        """Cleanup resources."""
        pass
