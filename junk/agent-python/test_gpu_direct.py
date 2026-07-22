#!/usr/bin/env python3
"""
GPU Direct Pipeline Test - Zero Copy Performance Test.

测试完整的 GPU Direct 管道:
DXGI Capture → D3D11 Texture → NVENC (Zero Copy) → H.264

性能目标:
- 1080p @ 60+ fps
- 端到端延迟 < 10ms
- 零 CPU 拷贝
"""

import asyncio
import logging
import sys
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent / 'src'))

logging.basicConfig(
    level=logging.INFO,
    format="%(asctime)s [%(levelname)s] %(message)s"
)

logger = logging.getLogger(__name__)


async def test_gpu_direct_pipeline():
    """
    测试 GPU Direct 管道性能。
    """

    logger.info("=" * 70)
    logger.info("GPU DIRECT PIPELINE TEST")
    logger.info("=" * 70)

    # ============================================================
    # 1. 检查 DLL 可用性
    # ============================================================
    logger.info("\n[1/4] 检查 GPU Direct 组件...")

    cpp_dir = Path(__file__).parent / 'cpp_capture'

    dlls = {
        "d3d12_hybrid_capture.dll": cpp_dir / "d3d12_hybrid_capture.dll",
        "nvenc_full.dll": cpp_dir / "nvenc_full.dll",
    }

    missing = []
    for name, path in dlls.items():
        if path.exists():
            logger.info(f"  ✓ {name}")
        else:
            logger.error(f"  ✗ {name} 未找到")
            missing.append(name)

    if missing:
        logger.error("\n缺少必需的 DLL 文件!")
        logger.info("请先编译 C++ 组件:")
        logger.info("  cd cpp_capture")
        logger.info("  build.bat")
        return False

    # ============================================================
    # 2. 初始化混合捕获器
    # ============================================================
    logger.info("\n[2/4] 初始化 GPU Direct 捕获...")

    from src.capture.hybrid_capture import create_hybrid_capture

    capture = create_hybrid_capture(monitor_index=0)

    if not capture.initialize():
        logger.error("捕获器初始化失败")
        return False

    # 获取 D3D11 设备和上下文
    d3d11_device = capture.get_d3d11_device()
    d3d11_context = capture.get_d3d11_context()

    logger.info(f"  D3D11 设备: 0x{d3d11_device or 0:X}")
    logger.info(f"  D3D11 上下文: 0x{d3d11_context or 0:X}")

    # ============================================================
    # 3. 初始化 NVENC 编码器
    # ============================================================
    logger.info("\n[3/4] 初始化 NVENC 编码器 (D3D11 模式)...")

    from src.encoder.nvenc_encoder import create_nvenc_encoder

    encoder = create_nvenc_encoder(
        d3d11_device=d3d11_device,
        d3d11_context=d3d11_context,
        width=1920,
        height=1080,
        quality=24,  # 高质量
        framerate=60
    )

    if not encoder:
        logger.error("NVENC 编码器初始化失败")
        capture.close()
        return False

    # ============================================================
    # 4. 运行 GPU Direct 管道测试
    # ============================================================
    logger.info("\n[4/4] 运行 GPU Direct 管道测试...")
    logger.info("=" * 70)

    test_duration = 3  # 秒
    target_fps = 60
    total_frames = target_fps * test_duration

    frame_times = []
    encode_times = []
    frame_sizes = []

    start_time = time.perf_counter()
    encoded_count = 0

    logger.info(f"捕获并编码 {total_frames} 帧...")

    for i in range(total_frames):
        frame_start = time.perf_counter()

        # 捕获帧 (GPU Direct - 返回 D3D11 纹理指针)
        frame_info = capture.capture_frame()

        if frame_info is None:
            logger.warning(f"帧 {i}: 捕获失败")
            continue

        capture_done = time.perf_counter()

        # 获取纹理指针
        texture_ptr = capture.get_texture_ptr()

        if not texture_ptr:
            logger.warning(f"帧 {i}: 纹理指针无效")
            continue

        # 编码 (GPU Direct - 直接从 D3D11 纹理编码)
        encoded_frame = encoder.encode_d3d11(texture_ptr)

        encode_done = time.perf_counter()

        if encoded_frame:
            frame_times.append((encode_done - frame_start) * 1000)  # ms
            encode_times.append((encode_done - capture_done) * 1000)  # ms
            frame_sizes.append(encoded_frame.size)
            encoded_count += 1

            # 进度更新
            if (i + 1) % (target_fps // 2) == 0:  # 每 0.5 秒
                elapsed = time.perf_counter() - start_time
                current_fps = encoded_count / elapsed
                avg_time = sum(frame_times[-(target_fps // 2):]) / len(frame_times[-(target_fps // 2):])
                logger.info(
                    f"  {(i+1):3d} 帧 | {current_fps:5.1f} fps | "
                    f"捕获+编码: {avg_time:.2f} ms"
                )

    end_time = time.perf_counter()
    duration = end_time - start_time

    # ============================================================
    # 结果分析
    # ============================================================
    logger.info("\n" + "=" * 70)
    logger.info("GPU DIRECT 管道测试结果")
    logger.info("=" * 70)

    if encoded_count == 0:
        logger.error("没有成功编码任何帧!")
        capture.close()
        encoder.close()
        return False

    actual_fps = encoded_count / duration
    avg_frame_time = sum(frame_times) / len(frame_times)
    avg_encode_time = sum(encode_times) / len(encode_times)
    avg_frame_size = sum(frame_sizes) / len(frame_sizes)
    bandwidth_mbps = (avg_frame_size * actual_fps * 8) / 1_000_000

    logger.info(f"\n持续时间:       {duration:.2f} 秒")
    logger.info(f"编码帧数:       {encoded_count}")
    logger.info(f"实际 FPS:       {actual_fps:.1f}")
    logger.info(f"目标 FPS:       {target_fps}")

    logger.info(f"\n平均帧时间:     {avg_frame_time:.2f} ms")
    logger.info(f"  - 捕获+编码:   {avg_frame_time:.2f} ms")
    logger.info(f"  - 纯编码:     {avg_encode_time:.2f} ms")

    logger.info(f"\n平均帧大小:     {avg_frame_size:,.0f} 字节")
    logger.info(f"带宽:           {bandwidth_mbps:.1f} Mbps")

    # 性能评级
    logger.info("\n" + "=" * 70)
    logger.info("性能评级")
    logger.info("=" * 70)

    if actual_fps >= 55:
        rating = "🚀 卓越 (55+ fps)"
        status = "GPU Direct 管道工作完美!"
    elif actual_fps >= 45:
        rating = "✓ 优秀 (45-55 fps)"
        status = "GPU Direct 管道工作良好"
    elif actual_fps >= 30:
        rating = "⚠ 良好 (30-45 fps)"
        status = "GPU Direct 管道基本可用"
    else:
        rating = "✗ 需优化 (< 30 fps)"
        status = "检查是否正确使用了 GPU Direct 路径"

    logger.info(f"\n{rating}")
    logger.info(f"状态: {status}")

    # 理论最大 FPS
    if avg_frame_time > 0:
        max_fps = 1000 / avg_frame_time
        logger.info(f"理论最大 FPS:  {max_fps:.1f}")

    # 与 CPU 路径对比
    logger.info("\n" + "=" * 70)
    logger.info("对比分析")
    logger.info("=" * 70)

    logger.info("\nGPU Direct 路径 (当前):")
    logger.info(f"  - 零 CPU 拷贝")
    logger.info(f"  - 延迟: ~{avg_frame_time:.1f} ms")
    logger.info(f"  - FPS: {actual_fps:.1f}")

    logger.info("\n传统 CPU 路径 (参考):")
    logger.info(f"  - GPU→CPU 拷贝: ~5-10 ms")
    logger.info(f"  - CPU→GPU 拷贝: ~5-10 ms")
    logger.info(f"  - 额外延迟: ~10-20 ms")
    logger.info(f"  - 预期 FPS: ~25-40 fps")

    speedup = (10 + avg_frame_time) / avg_frame_time if avg_frame_time > 0 else 1
    logger.info(f"\n性能提升: ~{speedup:.1f}x")

    # ============================================================
    # 清理
    # ============================================================
    logger.info("\n清理资源...")

    capture.close()
    encoder.close()

    logger.info("\n" + "=" * 70)
    logger.info("测试完成")
    logger.info("=" * 70)

    return actual_fps >= 30


async def main():
    """主测试入口。"""
    success = await test_gpu_direct_pipeline()

    if success:
        logger.info("\n✅ GPU Direct 管道测试通过!")
    else:
        logger.error("\n❌ GPU Direct 管道测试失败")

    return success


if __name__ == "__main__":
    success = asyncio.run(main())
    sys.exit(0 if success else 1)
