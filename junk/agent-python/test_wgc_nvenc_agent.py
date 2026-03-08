#!/usr/bin/env python3
"""
测试集成后的 WGC + NVENC Agent
"""

import asyncio
import logging
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent / 'src'))

from src.config import AgentConfig
from src.nvenc_agent import NVENCAgent

logging.basicConfig(
    level=logging.INFO,
    format="%(asctime)s [%(levelname)s] %(name)s: %(message)s"
)

logger = logging.getLogger(__name__)


async def test_wgc_nvenc_agent():
    """测试 WGC + NVENC Agent 集成"""

    print("=" * 70)
    print("WGC + NVENC Agent 集成测试")
    print("=" * 70)
    print()

    # 创建配置
    config = AgentConfig()
    config.capture.fps = 60
    config.monitor_index = 0
    config.quality = 20  # QP 20

    # 可选: 测试窗口捕获
    # config.capture_mode = 'window'
    # config.capture_target = 0x1900F2A  # 替换为实际 HWND

    print(f"配置:")
    print(f"  帧率: {config.framerate} fps")
    print(f"  监视器: {config.monitor_index}")
    print(f"  质量 (QP): {config.quality}")
    print(f"  捕获模式: {getattr(config, 'capture_mode', 'monitor')}")
    print()

    # 创建 Agent
    agent = NVENCAgent(config)

    print("[1/3] 初始化 Agent...")
    print("-" * 70)

    if not await agent.initialize():
        print("✗ Agent 初始化失败")
        return False

    print("✓ Agent 初始化成功")
    print(f"  分辨率: {agent._width}x{agent._height}")
    print(f"  D3D11 设备: {hex(agent._d3d11_device) if agent._d3d11_device else 'None'}")
    print()

    # 测试捕获循环
    print("[2/3] 测试捕获循环 (5秒)...")
    print("-" * 70)

    agent._running = True
    frame_count = 0
    encode_times = []

    buffer_size = agent._width * agent._height * 4
    import ctypes
    buffer = (ctypes.c_ubyte * buffer_size)()

    start_time = asyncio.get_event_loop().time()
    end_time = start_time + 5.0

    while asyncio.get_event_loop().time() < end_time and agent._running:
        loop_start = asyncio.get_event_loop().time()

        # 捕获帧
        frame = agent._wgc_capture.capture_frame()
        if frame:
            # 复制到 CPU
            if agent._wgc_capture.copy_to_cpu(buffer):
                frame_count += 1
                capture_time = (asyncio.get_event_loop().time() - loop_start) * 1000
                encode_times.append(capture_time)

                if frame_count <= 5 or frame_count % 30 == 0:
                    print(f"  帧 {frame_count}: 捕获 {capture_time:.2f}ms")

        await asyncio.sleep(0.001)  # 避免CPU占用过高

    actual_duration = asyncio.get_event_loop().time() - start_time
    actual_fps = frame_count / actual_duration if actual_duration > 0 else 0

    print()
    print(f"  捕获统计:")
    print(f"    总帧数: {frame_count}")
    print(f"    实际时间: {actual_duration:.2f} 秒")
    print(f"    实际 FPS: {actual_fps:.1f}")

    if encode_times:
        avg_capture = sum(encode_times) / len(encode_times)
        print(f"    平均捕获延迟: {avg_capture:.2f} ms")

    # 清理
    print()
    print("[3/3] 清理资源...")
    print("-" * 70)

    agent._running = False
    if agent._wgc_capture:
        agent._wgc_capture.stop()

    print("✓ 资源已释放")
    print()

    # 评级
    print("=" * 70)
    print("测试结果")
    print("=" * 70)

    if actual_fps >= 120:
        rating = "🚀 A+ - 超过 120fps!"
    elif actual_fps >= 60:
        rating = "✓ A - 优秀 (超过 60fps)"
    elif actual_fps >= 30:
        rating = "⚠ B - 良好"
    else:
        rating = "✗ C - 需优化"

    print(f"  评级: {rating}")
    print()
    print("WGC + NVENC Agent 集成测试完成!")

    return True


if __name__ == "__main__":
    try:
        result = asyncio.run(test_wgc_nvenc_agent())
        sys.exit(0 if result else 1)
    except KeyboardInterrupt:
        print("\n测试中断")
        sys.exit(1)
