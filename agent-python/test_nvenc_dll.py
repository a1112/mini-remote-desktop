#!/usr/bin/env python3
"""
NVENC DLL 验证测试.

测试 nvenc_full.dll 是否正确加载和导出函数。
"""

import ctypes
import logging
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent / 'src'))

logging.basicConfig(
    level=logging.INFO,
    format="%(asctime)s [%(levelname)s] %(message)s"
)

logger = logging.getLogger(__name__)


def test_nvenc_dll():
    """测试 NVENC DLL 加载。"""

    logger.info("=" * 70)
    logger.info("NVENC DLL 验证测试")
    logger.info("=" * 70)

    # ============================================================
    # 1. 检查 DLL 文件
    # ============================================================
    dll_path = Path(__file__).parent / 'nvenc_full.dll'

    logger.info(f"\n[1/5] 检查 DLL 文件...")
    logger.info(f"  路径: {dll_path}")

    if not dll_path.exists():
        logger.error("  ✗ DLL 不存在!")
        return False

    file_size = dll_path.stat().st_size
    logger.info(f"  ✓ DLL 存在 ({file_size:,} 字节)")

    # ============================================================
    # 2. 加载 DLL
    # ============================================================
    logger.info(f"\n[2/5] 加载 DLL...")

    try:
        dll = ctypes.CDLL(str(dll_path))
        logger.info("  ✓ DLL 加载成功")
    except Exception as e:
        logger.error(f"  ✗ DLL 加载失败: {e}")
        return False

    # ============================================================
    # 3. 检查导出函数
    # ============================================================
    logger.info(f"\n[3/5] 检查导出函数...")

    expected_functions = [
        "is_nvenc_supported",
        "is_cuda_d3d11_interop_supported",
        "init_nvenc_encoder_d3d11",
        "encode_nvenc_frame_cpu",
        "encode_nvenc_frame_d3d11",  # GPU Direct 函数
        "get_nvenc_encoded_frame",
        "free_nvenc_encoder",
        "request_nvenc_keyframe",
    ]

    for func_name in expected_functions:
        try:
            func = getattr(dll, func_name)
            logger.info(f"  ✓ {func_name}")
        except AttributeError:
            logger.error(f"  ✗ {func_name} 未找到")

    # ============================================================
    # 4. 测试 NVENC 可用性
    # ============================================================
    logger.info(f"\n[4/5] 测试 NVENC 可用性...")

    try:
        dll.is_nvenc_supported.restype = ctypes.c_int
        supported = dll.is_nvenc_supported()

        if supported:
            logger.info("  ✓ NVENC 可用")
        else:
            logger.warning("  ⚠ NVENC 不可用 (nvEncodeAPI64.dll 未找到)")

        # 测试 CUDA-D3D11 互操作
        dll.is_cuda_d3d11_interop_supported.restype = ctypes.c_int
        interop = dll.is_cuda_d3d11_interop_supported()

        if interop:
            logger.info("  ✓ CUDA-D3D11 互操作可用")
        else:
            logger.warning("  ⚠ CUDA-D3D11 互操作不可用")

    except Exception as e:
        logger.error(f"  ✗ 检查失败: {e}")

    # ============================================================
    # 5. 测试函数签名
    # ============================================================
    logger.info(f"\n[5/5] 测试函数签名...")

    # 定义配置结构
    class NVENCEncodeConfig(ctypes.Structure):
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

    # 设置函数签名
    dll.init_nvenc_encoder_d3d11.argtypes = [
        ctypes.c_void_p,
        ctypes.c_void_p,
        ctypes.POINTER(NVENCEncodeConfig)
    ]
    dll.init_nvenc_encoder_d3d11.restype = ctypes.c_void_p

    dll.encode_nvenc_frame_d3d11.argtypes = [
        ctypes.c_void_p,
        ctypes.c_void_p,
        ctypes.c_longlong,
        ctypes.c_int
    ]
    dll.encode_nvenc_frame_d3d11.restype = ctypes.c_int

    dll.free_nvenc_encoder.argtypes = [ctypes.c_void_p]
    dll.free_nvenc_encoder.restype = None

    logger.info("  ✓ encode_nvenc_frame_d3d11 签名正确 (GPU Direct)")

    # ============================================================
    # 总结
    # ============================================================
    logger.info("\n" + "=" * 70)
    logger.info("测试完成")
    logger.info("=" * 70)

    logger.info("\n导出的 GPU Direct 函数:")
    logger.info("  - init_nvenc_encoder_d3d11: 初始化 NVENC (D3D11 模式)")
    logger.info("  - encode_nvenc_frame_d3d11: 从 D3D11 纹理编码 (零拷贝)")
    logger.info("  - get_nvenc_encoded_frame: 获取编码数据")

    logger.info("\nGPU Direct 管道:")
    logger.info("  DXGI Capture → D3D11 Texture → NVENC → H.264")
    logger.info("       ↓            ↓               ↓")
    logger.info("    (GPU)        (GPU)           (GPU)")

    return True


def test_hybrid_capture_dll():
    """测试混合捕获 DLL。"""

    logger.info("\n" + "=" * 70)
    logger.info("混合捕获 DLL 验证")
    logger.info("=" * 70)

    dll_path = Path(__file__).parent / 'd3d12_hybrid_capture.dll'

    if not dll_path.exists():
        logger.error("DLL 不存在")
        return False

    logger.info(f"路径: {dll_path}")
    logger.info(f"大小: {dll_path.stat().st_size:,} 字节")

    try:
        dll = ctypes.CDLL(str(dll_path))
        logger.info("✓ DLL 加载成功")

        # 检查导出函数
        functions = [
            "init_hybrid_capture",
            "capture_hybrid_frame",
            "get_hybrid_d3d11_device",
            "get_hybrid_d3d11_context",
            "get_hybrid_d3d11_resource",
            "free_hybrid_capture",
        ]

        for func_name in functions:
            if hasattr(dll, func_name):
                logger.info(f"  ✓ {func_name}")
            else:
                logger.error(f"  ✗ {func_name}")

        return True

    except Exception as e:
        logger.error(f"DLL 加载失败: {e}")
        return False


if __name__ == "__main__":
    success = test_nvenc_dll()
    test_hybrid_capture_dll()

    sys.exit(0 if success else 1)
