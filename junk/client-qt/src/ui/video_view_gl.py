"""
Hardware-accelerated OpenGL video display widget.

Uses OpenGL for GPU-based rendering with lower CPU usage.
"""

import logging
from typing import Optional

from PySide6.QtCore import Qt, QObject, Signal, Slot
from PySide6.QtGui import QImage, QPainter, QPen, QColor, QSurfaceFormat
from PySide6.QtOpenGLWidgets import QOpenGLWidget
from PySide6.QtWidgets import QWidget, QSizePolicy
from PySide6.QtOpenGL import QOpenGLTexture
from PySide6.QtOpenGLWidgets import QOpenGLWidget

import numpy as np
import numpy.typing as npt

logger = logging.getLogger(__name__)


class FrameSignal(QObject):
    """Signal for thread-safe frame updates."""
    frame_ready = Signal(object)


class GLVideoView(QOpenGLWidget):
    """
    Hardware-accelerated OpenGL video view.

    Uses OpenGL textures for GPU-accelerated rendering.
    Much lower CPU usage than software rendering.
    """

    def __init__(self, parent=None):
        """Initialize OpenGL video view."""
        super().__init__(parent)

        self._frame: Optional[npt.NDArray[np.uint8]] = None
        self._texture: Optional[QOpenGLTexture] = None
        self._scale_mode = "fit"
        self._maintain_aspect_ratio = True

        # Frame signal
        self._frame_signal = FrameSignal()
        self._frame_signal.frame_ready.connect(self._on_frame_received)

        # Set OpenGL format
        fmt = QSurfaceFormat()
        fmt.setSamples(0)  # Disable anti-aliasing for performance
        fmt.setSwapInterval(0)  # Disable vsync for lowest latency
        self.setFormat(fmt)

        # Performance tracking
        self._frame_count = 0

    def initializeGL(self) -> None:
        """Initialize OpenGL resources."""
        gl = self.context().functions()
        gl.glClearColor(0.08, 0.08, 0.08, 1.0)
        logger.info("OpenGL renderer initialized")

    def resizeGL(self, w: int, h: int) -> None:
        """Handle resize."""
        gl = self.context().functions()
        gl.glViewport(0, 0, w, h)

    def paintGL(self) -> None:
        """Paint using OpenGL."""
        from PySide6.QtOpenGLWidgets import QOpenGLWidget
        gl = self.context().functions()

        if self._frame is None:
            gl.glClearColor(0.08, 0.08, 0.08, 1.0)
            gl.glClear(0x00004000)  # GL_COLOR_BUFFER_BIT
            return

        # Create or update texture
        if self._texture is None or self._texture.isDestroyed():
            self._create_texture()

        if self._texture and not self._texture.isDestroyed():
            # Upload frame data to GPU
            height, width = self._frame.shape[:2]

            self._texture.setData(
                QOpenGLTexture.RGB,
                QOpenGLTexture.UInt8,
                self._frame.data
            )

            # Draw textured quad
            self._draw_textured_quad()

    def _create_texture(self) -> None:
        """Create OpenGL texture."""
        if self._frame is None:
            return

        height, width = self._frame.shape[:2]

        self._texture = QOpenGLTexture(QOpenGLTexture.Target2D)
        self._texture.setFormat(QOpenGLTexture.RGBFormat)
        self._texture.setSize(width, height)
        self._texture.setMinificationFilter(QOpenGLTexture.Linear)
        self._texture.setMagnificationFilter(QOpenGLTexture.Linear)
        self._texture.setWrapMode(QOpenGLTexture.ClampToEdge)
        self._texture.allocateStorage()

    def _draw_textured_quad(self) -> None:
        """Draw a textured quad using legacy OpenGL (simpler)."""
        gl = self.context().functions()

        gl.glEnable(gl.GL_BLEND)
        gl.glBlendFunc(gl.GL_SRC_ALPHA, gl.GL_ONE_MINUS_SRC_ALPHA)

        gl.glEnable(gl.GL_TEXTURE_2D)
        if self._texture:
            self._texture.bind()

        # Calculate display rectangle
        view_w = self.width()
        view_h = self.height()
        frame_w, frame_h = self._frame.shape[1], self._frame.shape[0]

        x, y, w, h = self._calculate_display_rect(frame_w, frame_h, view_w, view_h)

        # Convert to normalized device coordinates
        nx = 2.0 * x / view_w - 1.0
        ny = 1.0 - 2.0 * y / view_h
        nw = 2.0 * w / view_w
        nh = 2.0 * h / view_h

        # Draw quad
        gl.glColor4f(1, 1, 1, 1)
        gl.glBegin(gl.GL_QUADS)
        gl.glTexCoord2f(0, 1); gl.glVertex2f(nx, ny)
        gl.glTexCoord2f(1, 1); gl.glVertex2f(nx + nw, ny)
        gl.glTexCoord2f(1, 0); gl.glVertex2f(nx + nw, ny + nh)
        gl.glTexCoord2f(0, 0); gl.glVertex2f(nx, ny + nh)
        gl.glEnd()

        if self._texture:
            self._texture.release()

        gl.glDisable(gl.GL_TEXTURE_2D)
        gl.glDisable(gl.GL_BLEND)

    def _calculate_display_rect(self, frame_w: int, frame_h: int, view_w: int, view_h: int):
        """Calculate display rectangle (x, y, w, h)."""
        frame_aspect = frame_w / frame_h if frame_h > 0 else 1.0
        view_aspect = view_w / view_h if view_h > 0 else 1.0

        if self._scale_mode == "stretch" or not self._maintain_aspect_ratio:
            return 0, 0, view_w, view_h

        if self._scale_mode == "fit":
            if view_aspect > frame_aspect:
                h = view_h
                w = int(view_h * frame_aspect)
                x = (view_w - w) // 2
                y = 0
            else:
                w = view_w
                h = int(view_w / frame_aspect)
                x = 0
                y = (view_h - h) // 2
        else:  # fill
            if view_aspect > frame_aspect:
                w = view_w
                h = int(view_w / frame_aspect)
                x = 0
                y = (view_h - h) // 2
            else:
                h = view_h
                w = int(view_h * frame_aspect)
                x = (view_w - w) // 2
                y = 0

        return x, y, w, h

    @Slot(object)
    def _on_frame_received(self, frame: npt.NDArray[np.uint8]) -> None:
        """Handle frame from signal."""
        self._frame = frame
        self._frame_count += 1
        self.update()

    def set_frame(self, frame: npt.NDArray[np.uint8]) -> None:
        """Set current video frame (thread-safe)."""
        self._frame_signal.frame_ready.emit(frame)

    def clear_frame(self) -> None:
        """Clear current frame."""
        self._frame = None
        if self._texture:
            self._texture.destroy()
            self._texture = None
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

    @property
    def frame_count(self) -> int:
        """Get number of frames rendered."""
        return self._frame_count

    def cleanup(self) -> None:
        """Cleanup OpenGL resources."""
        if self._texture:
            self._texture.destroy()
            self._texture = None


class VideoView(QWidget):
    """
    Video display widget with optional OpenGL rendering.

    Set use_opengl=True in constructor for GPU-accelerated rendering.
    """

    def __init__(self, parent=None, use_opengl: bool = False):
        """Initialize video view."""
        super().__init__(parent)

        self._use_opengl = use_opengl
        self._gl_view: Optional[GLVideoView] = None
        self._software_mode = False

        if use_opengl:
            try:
                self._gl_view = GLVideoView(self)
                self._gl_view.setSizePolicy(QSizePolicy.Expanding, QSizePolicy.Expanding)
                # Make the GL view fill this widget
                layout = QWidget.layout() or QWidget()
                from PySide6.QtWidgets import QVBoxLayout
                l = QVBoxLayout(self)
                l.setContentsMargins(0, 0, 0, 0)
                l.addWidget(self._gl_view)
                logger.info("Using OpenGL rendering backend")
            except Exception as e:
                logger.warning(f"OpenGL not available: {e}, using software rendering")
                self._use_opengl = False
                self._software_mode = True

        if not use_opengl or self._software_mode:
            self._setup_software_rendering()

    def _setup_software_rendering(self) -> None:
        """Setup software rendering fallback."""
        self._background_color = QColor(20, 20, 20)
        self._frame: Optional[npt.NDArray[np.uint8]] = None
        self._scale_mode = "fit"
        self._maintain_aspect_ratio = True

        self._frame_signal = FrameSignal()
        self._frame_signal.frame_ready.connect(self._on_frame_received)

        self.setSizePolicy(QSizePolicy.Expanding, QSizePolicy.Expanding)
        self.setMinimumSize(320, 240)
        self.setStyleSheet(f"background-color: {self._background_color.name()};")

    @Slot(object)
    def _on_frame_received(self, frame: npt.NDArray[np.uint8]) -> None:
        """Handle frame from signal."""
        if self._gl_view:
            self._gl_view.set_frame(frame)
        else:
            self._frame = frame
            self.update()

    def set_frame(self, frame: npt.NDArray[np.uint8]) -> None:
        """Set current video frame (thread-safe)."""
        self._frame_signal.frame_ready.emit(frame)

    def clear_frame(self) -> None:
        """Clear current frame."""
        if self._gl_view:
            self._gl_view.clear_frame()
        else:
            self._frame = None
            self.update()

    def set_scale_mode(self, mode: str) -> None:
        """Set scale mode."""
        if self._gl_view:
            self._gl_view.set_scale_mode(mode)
        else:
            self._scale_mode = mode
            self.update()

    def set_maintain_aspect_ratio(self, maintain: bool) -> None:
        """Set whether to maintain aspect ratio."""
        if self._gl_view:
            self._gl_view.set_maintain_aspect_ratio(maintain)
        else:
            self._maintain_aspect_ratio = maintain
            self.update()

    def paintEvent(self, event) -> None:
        """Paint event for software rendering."""
        if self._gl_view:
            return  # OpenGL widget handles rendering

        super().paintEvent(event)

        painter = QPainter(self)
        painter.setRenderHint(QPainter.Antialiasing)
        painter.setRenderHint(QPainter.SmoothPixmapTransform)

        painter.fillRect(self.rect(), self._background_color)

        if self._frame is None:
            self._paint_placeholder(painter)
            return

        height, width = self._frame.shape[:2]
        channels = self._frame.shape[2] if len(self._frame.shape) == 3 else 1

        if channels == 3:
            fmt = QImage.Format.Format_RGB888
        elif channels == 4:
            fmt = QImage.Format.Format_RGBA8888
        elif channels == 1:
            fmt = QImage.Format.Format_Grayscale8
        else:
            self._paint_placeholder(painter)
            return

        image = QImage(
            self._frame.data,
            width,
            height,
            self._frame.strides[0] if hasattr(self._frame, 'strides') else width * channels,
            fmt
        )

        if not image.isNull():
            dest_rect = self._calculate_display_rect(
                image.width(), image.height(),
                self.width(), self.height()
            )
            painter.drawImage(dest_rect, image)

    def _paint_placeholder(self, painter: QPainter) -> None:
        """Paint placeholder."""
        rect = self.rect()
        pen = QPen(QColor(60, 60, 60), 2)
        painter.setPen(pen)
        painter.drawRect(rect.adjusted(1, 1, -1, -1))
        painter.setPen(QColor(120, 120, 120))
        font = painter.font()
        font.setPointSize(14)
        painter.setFont(font)
        text = "No Connection"
        metrics = painter.fontMetrics()
        text_rect = metrics.boundingRect(text)
        text_rect.moveCenter(rect.center())
        painter.drawText(text_rect, Qt.AlignmentFlag.AlignCenter, text)

    def _calculate_display_rect(self, frame_w: int, frame_h: int, view_w: int, view_h: int):
        """Calculate display rect."""
        from PySide6.QtCore import QRect

        if self._scale_mode == "stretch" or not self._maintain_aspect_ratio:
            return QRect(0, 0, view_w, view_h)

        frame_aspect = frame_w / frame_h if frame_h > 0 else 1.0
        view_aspect = view_w / view_h if view_h > 0 else 1.0

        if self._scale_mode == "fit":
            if view_aspect > frame_aspect:
                h = view_h
                w = int(view_h * frame_aspect)
                x = (view_w - w) // 2
                y = 0
            else:
                w = view_w
                h = int(view_w / frame_aspect)
                x = 0
                y = (view_h - h) // 2
        else:
            if view_aspect > frame_aspect:
                w = view_w
                h = int(view_w / frame_aspect)
                x = 0
                y = (view_h - h) // 2
            else:
                h = view_h
                w = int(view_h * frame_aspect)
                x = (view_w - w) // 2
                y = 0

        return QRect(x, y, w, h)

    def cleanup(self) -> None:
        """Cleanup resources."""
        if self._gl_view:
            self._gl_view.cleanup()
