"""
Video view backend factory.

Selects a display widget backend based on config and platform:
- d3d11: Windows preferred path (Qt ANGLE / D3D11-backed composition)
- opengl: QOpenGL-based view
- gpu: QWidget + pixmap GPU-accelerated path
- software: pure software QImage path
"""

from __future__ import annotations

import logging
import platform
from typing import Tuple

logger = logging.getLogger(__name__)


def create_video_view(renderer: str = "auto"):
    """
    Create video view widget and return (widget, backend_name).
    """
    pref = (renderer or "auto").strip().lower()
    is_windows = platform.system().lower() == "windows"

    # Windows preferred path: run through Qt on ANGLE/D3D11 when available.
    if pref in ("auto", "d3d11") and is_windows:
        try:
            from .video_view_gpu import GPUVideoView

            logger.info("video backend selected: d3d11(angle)+gpu")
            return GPUVideoView(), "d3d11(angle)+gpu"
        except Exception as e:
            logger.warning("d3d11 backend init failed: %s", e)
            if pref == "d3d11":
                # Explicit d3d11 request should still fallback gracefully.
                logger.warning("fallback from d3d11 to software")

    if pref in ("auto", "opengl"):
        try:
            from .video_view_gl import VideoView as GLVideoView

            logger.info("video backend selected: opengl")
            return GLVideoView(use_opengl=True), "opengl"
        except Exception as e:
            logger.warning("opengl backend init failed: %s", e)

    if pref in ("auto", "gpu"):
        try:
            from .video_view_gpu import GPUVideoView

            logger.info("video backend selected: gpu")
            return GPUVideoView(), "gpu"
        except Exception as e:
            logger.warning("gpu backend init failed: %s", e)

    from .video_view import VideoView as SoftwareVideoView

    logger.info("video backend selected: software")
    return SoftwareVideoView(), "software"

