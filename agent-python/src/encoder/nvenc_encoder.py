"""
NVENC H.264 hardware encoder wrapper.

Provides ctypes interface to nvenc_full.dll for hardware-accelerated encoding.
"""

import ctypes
import logging
import numpy as np
from pathlib import Path
from typing import Optional, Tuple
from dataclasses import dataclass

logger = logging.getLogger(__name__)


@dataclass
class NVENCEncodedFrame:
    """Encoded frame data from NVENC."""
    data: bytes
    size: int
    key_frame: bool
    timestamp: int


@dataclass
class NVENCConfig:
    """NVENC encoder configuration."""
    width: int
    height: int
    framerate: int = 60
    bitrate: int = 10_000_000  # For CBR mode (not used in ConstQP)
    gop_size: int = 60
    preset: int = 3  # 0=default, 1=slow, 2=medium, 3=fast, 4=fastest
    rc_mode: int = 0  # 0=ConstQP, 1=VBR, 2=CBR, 3=CQ
    quality: int = 24  # QP value (1-51, lower is better)


class NVENCEncoder:
    """
    NVENC H.264 hardware encoder.

    Uses nvenc_full.dll for GPU-accelerated encoding with D3D11-CUDA interop.
    """

    # Quality presets
    QUALITY_FIDELITY = 18   # ~200 Mbps
    QUALITY_HIGH = 24       # ~80 Mbps
    QUALITY_MEDIUM_HIGH = 30  # ~50 Mbps
    QUALITY_MEDIUM = 36     # ~35 Mbps
    QUALITY_LOW = 42        # ~30 Mbps
    QUALITY_VERY_LOW = 48   # ~18 Mbps

    def __init__(self, d3d11_device, d3d11_context, config: NVENCConfig):
        """
        Initialize NVENC encoder.

        Args:
            d3d11_device: D3D11 device pointer (from hybrid capture)
            d3d11_context: D3D11 context pointer (from hybrid capture)
            config: Encoder configuration
        """
        self.config = config
        self._d3d11_device = d3d11_device
        self._d3d11_context = d3d11_context
        self._handle: Optional[ctypes.c_void_p] = None
        self._dll: Optional[ctypes.CDLL] = None
        self._frame_counter = 0

    def initialize(self) -> bool:
        """
        Initialize the NVENC encoder.

        Returns:
            True if successful, False otherwise
        """
        dll_path = Path(__file__).parent.parent.parent / 'cpp_capture' / 'nvenc_full.dll'
        if not dll_path.exists():
            logger.error(f"NVENC DLL not found: {dll_path}")
            return False

        try:
            self._dll = ctypes.CDLL(str(dll_path))

            # Setup function signatures
            self._setup_function_signatures()

            # Initialize encoder
            config_struct = self._create_config_struct()

            self._handle = self._dll.init_nvenc_encoder_d3d11(
                ctypes.c_void_p(self._d3d11_device),
                ctypes.c_void_p(self._d3d11_context),
                ctypes.byref(config_struct)
            )

            if not self._handle:
                logger.error("Failed to initialize NVENC encoder")
                return False

            logger.info(f"NVENC encoder initialized: {self.config.width}x{self.config.height} "
                       f"@ {self.config.framerate}fps, QP={self.config.quality}")
            return True

        except Exception as e:
            logger.error(f"Failed to load NVENC DLL: {e}")
            return False

    def _setup_function_signatures(self) -> None:
        """Setup ctypes function signatures."""
        if not self._dll:
            return

        class NVENCEncodeConfigStruct(ctypes.Structure):
            _fields_ = [
                ("width", ctypes.c_int),
                ("height", ctypes.c_int),
                ("framerate", ctypes.c_int),
                ("bitrate", ctypes.c_int),
                ("gop_size", ctypes.c_int),
                ("preset", ctypes.c_int),
                ("rc_mode", ctypes.c_int),
                ("quality", ctypes.c_int),
            ]

        class NVENCEncodedFrameStruct(ctypes.Structure):
            _fields_ = [
                ("data", ctypes.POINTER(ctypes.c_ubyte)),
                ("size", ctypes.c_int),
                ("key_frame", ctypes.c_int),
                ("timestamp", ctypes.c_longlong),
            ]

        self._dll.init_nvenc_encoder_d3d11.argtypes = [
            ctypes.c_void_p,
            ctypes.c_void_p,
            ctypes.POINTER(NVENCEncodeConfigStruct)
        ]
        self._dll.init_nvenc_encoder_d3d11.restype = ctypes.c_void_p

        self._dll.encode_nvenc_frame_cpu.argtypes = [
            ctypes.c_void_p,
            ctypes.POINTER(ctypes.c_ubyte),
            ctypes.c_int,
            ctypes.c_longlong,
            ctypes.c_int
        ]
        self._dll.encode_nvenc_frame_cpu.restype = ctypes.c_int

        # D3D11 GPU Direct encoding
        self._dll.encode_nvenc_frame_d3d11.argtypes = [
            ctypes.c_void_p,
            ctypes.c_void_p,
            ctypes.c_longlong,
            ctypes.c_int
        ]
        self._dll.encode_nvenc_frame_d3d11.restype = ctypes.c_int

        self._dll.get_nvenc_encoded_frame.argtypes = [
            ctypes.c_void_p,
            ctypes.POINTER(NVENCEncodedFrameStruct)
        ]
        self._dll.get_nvenc_encoded_frame.restype = ctypes.c_int

        self._dll.free_nvenc_encoder.argtypes = [ctypes.c_void_p]
        self._dll.free_nvenc_encoder.restype = None

        self._dll.request_nvenc_keyframe.argtypes = [ctypes.c_void_p]
        self._dll.request_nvenc_keyframe.restype = None

        # Store struct classes for later use
        self._ConfigStruct = NVENCEncodeConfigStruct
        self._FrameStruct = NVENCEncodedFrameStruct

    def _create_config_struct(self) -> '_ConfigStruct':
        """Create configuration struct for NVENC."""
        if not hasattr(self, '_ConfigStruct'):
            raise RuntimeError("Function signatures not setup")

        return self._ConfigStruct(
            width=self.config.width,
            height=self.config.height,
            framerate=self.config.framerate,
            bitrate=self.config.bitrate,
            gop_size=self.config.gop_size,
            preset=self.config.preset,
            rc_mode=self.config.rc_mode,
            quality=self.config.quality,
        )

    def encode(self, frame_bgra: bytes) -> Optional[NVENCEncodedFrame]:
        """
        Encode a BGRA frame.

        Args:
            frame_bgra: BGRA frame data (width * height * 4 bytes)

        Returns:
            Encoded frame data, or None if encoding failed
        """
        if not self._handle or not self._dll:
            return None

        # Convert bytes to ctypes pointer
        buffer = (ctypes.c_ubyte * len(frame_bgra)).from_buffer_copy(frame_bgra)

        # Encode frame
        timestamp = self._frame_counter
        force_keyframe = 1 if self._frame_counter == 0 else 0

        result = self._dll.encode_nvenc_frame_cpu(
            self._handle,
            buffer,
            len(frame_bgra),
            timestamp,
            force_keyframe
        )

        if result != 1:
            logger.warning(f"Encode failed: {result}")
            return None

        # Get encoded frame
        frame_info = self._FrameStruct()
        result = self._dll.get_nvenc_encoded_frame(
            self._handle,
            ctypes.byref(frame_info)
        )

        if result != 1 or frame_info.size == 0:
            return None

        # Copy encoded data
        data = bytes(ctypes.string_at(frame_info.data, frame_info.size))

        self._frame_counter += 1

        return NVENCEncodedFrame(
            data=data,
            size=frame_info.size,
            key_frame=bool(frame_info.key_frame),
            timestamp=frame_info.timestamp
        )

    def encode_d3d11(self, d3d11_texture_ptr: int) -> Optional[NVENCEncodedFrame]:
        """
        Encode a D3D11 texture directly (GPU Direct / Zero Copy).

        This is the fastest encoding path - no CPU memory copy involved.
        The texture stays entirely on the GPU.

        Args:
            d3d11_texture_ptr: Pointer to ID3D11Texture2D (as integer)

        Returns:
            Encoded frame data, or None if encoding failed
        """
        if not self._handle or not self._dll:
            return None

        # Encode frame directly from D3D11 texture
        timestamp = self._frame_counter
        force_keyframe = 1 if self._frame_counter == 0 else 0

        result = self._dll.encode_nvenc_frame_d3d11(
            self._handle,
            ctypes.c_void_p(d3d11_texture_ptr),
            timestamp,
            force_keyframe
        )

        if result != 1:
            logger.warning(f"D3D11 encode failed: {result}")
            return None

        # Get encoded frame
        frame_info = self._FrameStruct()
        result = self._dll.get_nvenc_encoded_frame(
            self._handle,
            ctypes.byref(frame_info)
        )

        if result != 1 or frame_info.size == 0:
            return None

        # Copy encoded data (this is the only CPU copy)
        data = bytes(ctypes.string_at(frame_info.data, frame_info.size))

        self._frame_counter += 1

        return NVENCEncodedFrame(
            data=data,
            size=frame_info.size,
            key_frame=bool(frame_info.key_frame),
            timestamp=frame_info.timestamp
        )

    def encode_numpy(self, frame_array: np.ndarray) -> Optional[NVENCEncodedFrame]:
        """
        Encode a numpy array frame (BGRA format).

        Args:
            frame_array: numpy array with shape (height, width, 4)

        Returns:
            Encoded frame data, or None if encoding failed
        """
        # Ensure contiguous array
        frame_contiguous = np.ascontiguousarray(frame_array)
        return self.encode(frame_contiguous.tobytes())

    def request_keyframe(self) -> None:
        """Request the next frame to be a keyframe."""
        if self._handle and self._dll:
            self._dll.request_nvenc_keyframe(self._handle)

    def close(self) -> None:
        """Release NVENC encoder resources."""
        if self._handle and self._dll:
            self._dll.free_nvenc_encoder(self._handle)
            self._handle = None
            logger.info("NVENC encoder closed")


def create_nvenc_encoder(
    d3d11_device,
    d3d11_context,
    width: int,
    height: int,
    quality: int = NVENCEncoder.QUALITY_HIGH,
    framerate: int = 60
) -> Optional[NVENCEncoder]:
    """
    Create an NVENC encoder with default settings.

    Args:
        d3d11_device: D3D11 device pointer
        d3d11_context: D3D11 context pointer
        width: Frame width
        height: Frame height
        quality: QP value (18-51, lower is better)
        framerate: Target framerate

    Returns:
        NVENCEncoder instance or None if initialization failed
    """
    config = NVENCConfig(
        width=width,
        height=height,
        framerate=framerate,
        quality=quality,
        rc_mode=0,  # ConstQP mode
        preset=3,   # Fast preset
    )

    encoder = NVENCEncoder(d3d11_device, d3d11_context, config)
    if encoder.initialize():
        return encoder
    return None
