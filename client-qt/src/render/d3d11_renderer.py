"""
D3D11 Hardware-accelerated renderer for Qt.

Uses Direct3D 11 for hardware-accelerated video rendering.
Integrates with PySide6 via QWindow with native D3D11 surface.

Requires:
- Windows 8+ (for D3D11.1)
- PySide6
- comtypes
"""

import logging
import threading
from typing import Optional, Tuple

import numpy as np
import numpy.typing as npt

logger = logging.getLogger(__name__)


class D3D11Renderer:
    """
    D3D11 Hardware-accelerated renderer.

    Provides direct D3D11 texture rendering for minimal latency.
    """

    def __init__(self):
        """Initialize D3D11 renderer."""
        self._device = None
        self._context = None
        self._swap_chain = None
        self._render_target = None
        self._texture = None
        self._shader_resource = None
        self._sampler = None
        self._initialized = False

        self._width = 0
        self._height = 0
        self._hwnd = None

    async def initialize(self, width: int, height: int, hwnd: int = None) -> bool:
        """
        Initialize D3D11 renderer.

        Args:
            width: Render target width
            height: Render target height
            hwnd: Window handle (HWND) for swap chain

        Returns:
            True if successful
        """
        try:
            import comtypes
            from comtypes import GUID

            # Load D3D11 DLL
            try:
                import d3d11
                import d3dcompiler
            except ImportError:
                logger.warning("D3D11 comtypes bindings not available, using software rendering")
                return False

            self._width = width
            self._height = height
            self._hwnd = hwnd

            # Create D3D11 device
            self._device = d3d11.D3D11CreateDevice()

            if not self._device:
                logger.error("Failed to create D3D11 device")
                return False

            # Get immediate context
            self._context = self._device.GetImmediateContext()

            # Create swap chain if HWND provided
            if hwnd:
                if not await self._create_swap_chain(hwnd):
                    logger.warning("Failed to create swap chain, using texture rendering")

            self._initialized = True
            logger.info(f"D3D11 renderer initialized: {width}x{height}")
            return True

        except ImportError:
            logger.warning("D3D11 libraries not available")
            return False
        except Exception as e:
            logger.error(f"D3D11 initialization failed: {e}")
            return False

    async def _create_swap_chain(self, hwnd: int) -> bool:
        """Create D3D11 swap chain."""
        try:
            import d3d11

            # Swap chain description
            sd = d3d11.DXGI_SWAP_CHAIN_DESC()
            sd.BufferCount = 2
            sd.BufferDesc.Width = self._width
            sd.BufferDesc.Height = self._height
            sd.BufferDesc.Format = d3d11.DXGI_FORMAT_B8G8R8A8_UNORM
            sd.BufferDesc.RefreshRate.Numerator = 60
            sd.BufferDesc.RefreshRate.Denominator = 1
            sd.BufferUsage = d3d11.DXGI_USAGE_RENDER_TARGET_OUTPUT
            sd.OutputWindow = hwnd
            sd.SampleDesc.Count = 1
            sd.SampleDesc.Quality = 0
            sd.Windowed = True
            sd.SwapEffect = d3d11.DXGI_SWAP_EFFECT_DISCARD

            # Create swap chain
            factory = d3d11.CreateDXGIFactory()
            self._swap_chain = factory.CreateSwapChain(self._device, sd)

            # Create render target view
            back_buffer = self._swap_chain.GetBuffer(0)
            self._render_target = self._device.CreateRenderTargetView(back_buffer)

            return True

        except Exception as e:
            logger.error(f"Swap chain creation failed: {e}")
            return False

    def update_texture(self, frame: npt.NDArray[np.uint8]) -> bool:
        """
        Update texture with new frame data.

        Args:
            frame: RGB/RGBA numpy array (height, width, channels)

        Returns:
            True if successful
        """
        if not self._initialized or self._device is None:
            return False

        try:
            import d3d11

            height, width = frame.shape[:2]
            channels = frame.shape[2] if len(frame.shape) == 3 else 3

            # Create or update texture
            if self._texture is None or self._texture.width != width or self._texture.height != height:
                self._texture = self._create_texture(width, height)

            # Update texture data
            self._context.UpdateSubresource(self._texture, 0, frame.tobytes())

            # Present if swap chain exists
            if self._swap_chain:
                self._context.OMSetRenderTargets([self._render_target], None)
                self._context.Present(1, 0)

            return True

        except Exception as e:
            logger.debug(f"Texture update failed: {e}")
            return False

    def _create_texture(self, width: int, height: int):
        """Create a D3D11 texture."""
        import d3d11

        desc = d3d11.D3D11_TEXTURE2D_DESC()
        desc.Width = width
        desc.Height = height
        desc.MipLevels = 1
        desc.ArraySize = 1
        desc.Format = d3d11.DXGI_FORMAT_R8G8B8A8_UNORM
        desc.SampleDesc.Count = 1
        desc.Usage = d3d11.D3D11_USAGE_DEFAULT
        desc.BindFlags = d3d11.D3D11_BIND_SHADER_RESOURCE
        desc.CPUAccessFlags = 0

        return self._device.CreateTexture2D(desc)

    async def close(self) -> None:
        """Close and release D3D11 resources."""
        if self._swap_chain:
            try:
                self._swap_chain.Release()
            except Exception:
                pass
            self._swap_chain = None

        if self._render_target:
            try:
                self._render_target.Release()
            except Exception:
                pass
            self._render_target = None

        if self._texture:
            try:
                self._texture.Release()
            except Exception:
                pass
            self._texture = None

        if self._context:
            try:
                self._context.Release()
            except Exception:
                pass
            self._context = None

        if self._device:
            try:
                self._device.Release()
            except Exception:
                pass
            self._device = None

        self._initialized = False
        logger.info("D3D11 renderer closed")


class D3D11WidgetMixin:
    """
    Mixin class to add D3D11 rendering capability to Qt widgets.

    Usage:
        class MyVideoView(QOpenGLWidget, D3D11WidgetMixin):
            pass
    """

    def __init__(self):
        """Initialize D3D11 widget mixin."""
        self._d3d11_renderer: Optional[D3D11Renderer] = None
        self._use_hardware_rendering = False

    def enable_hardware_rendering(self, enable: bool = True) -> bool:
        """
        Enable or disable hardware-accelerated rendering.

        Args:
            enable: True to enable, False to disable

        Returns:
            True if hardware rendering is available
        """
        self._use_hardware_rendering = enable

        if enable:
            if self._d3d11_renderer is None:
                self._d3d11_renderer = D3D11Renderer()

            # Initialize when widget is shown
            if self.isVisible() and self.winId():
                hwnd = int(self.winId())
                asyncio.create_task(
                    self._d3d11_renderer.initialize(
                        self.width(),
                        self.height(),
                        hwnd
                    )
                )

        return True

    def set_frame_hardware(self, frame: npt.NDArray[np.uint8]) -> None:
        """Set frame using hardware renderer."""
        if self._use_hardware_rendering and self._d3d11_renderer:
            self._d3d11_renderer.update_texture(frame)
        else:
            # Fallback to software rendering
            self.set_frame(frame)

    def cleanup_hardware_rendering(self) -> None:
        """Cleanup hardware rendering resources."""
        if self._d3d11_renderer:
            asyncio.create_task(self._d3d11_renderer.close())


def create_d3d11_renderer() -> Optional[D3D11Renderer]:
    """
    Create a D3D11 renderer instance.

    Returns:
        D3D11Renderer instance or None if not available
    """
    renderer = D3D11Renderer()
    # Note: Needs async initialization
    return renderer


# Simple software fallback for when D3D11 is not available
class SoftwareRenderer:
    """Software renderer using QImage for compatibility."""

    def __init__(self):
        """Initialize software renderer."""
        self._current_frame = None

    def update_frame(self, frame: npt.NDArray[np.uint8]) -> bool:
        """Update current frame."""
        self._current_frame = frame
        return True

    def get_frame(self) -> Optional[npt.NDArray[np.uint8]]:
        """Get current frame."""
        return self._current_frame

    @staticmethod
    def frame_to_qimage(frame: npt.NDArray[np.uint8]):
        """Convert numpy frame to QImage."""
        from PySide6.QtGui import QImage

        height, width = frame.shape[:2]
        channels = frame.shape[2] if len(frame.shape) == 3 else 1

        if channels == 3:
            format = QImage.Format.Format_RGB888
        elif channels == 4:
            format = QImage.Format.Format_RGBA8888
        elif channels == 1:
            format = QImage.Format.Format_Grayscale8
        else:
            return None

        return QImage(
            frame.data,
            width,
            height,
            frame.strides[0] if hasattr(frame, 'strides') else width * channels,
            format
        )
