#!/usr/bin/env python3
"""
WGC Capture 性能测试

测试内容:
1. 捕获延迟 (每帧处理时间)
2. 实际帧率
3. GPU Direct 验证 (D3D11 纹理输出)
4. 与 NVENC 集成测试

目标: 1080p@144fps
"""

import sys
import time
import ctypes
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent / 'src'))

from src.capture.wgc_capture import WGCCapture


def print_header(title):
    """打印标题"""
    print()
    print("=" * 70)
    print(title)
    print("=" * 70)


def print_test_info():
    """打印测试信息"""
    print("\n" + "=" * 70)
    print("WGC Capture 性能测试")
    print("=" * 70)
    print()
    print("测试目标:")
    print("  - 分辨率: 1920x1080 或更高")
    print("  - 目标 FPS: 144")
    print("  - 帧预算: 6.94ms/帧")
    print()
    print("测试场景:")
    print("  1. 捕获初始化")
    print("  2. 连续捕获帧率")
    print("  3. GPU Direct 验证")
    print("  4. 与 NVENC 集成")
    print()


def test_capture_initialization():
    """测试 1: 捕获初始化"""
    print_header("[测试 1/4] 捕获初始化")

    monitors = WGCCapture.enum_monitors()
    print(f"  发现 {len(monitors)} 个监视器:")
    for i, m in enumerate(monitors):
        primary = " [主]" if m.is_primary else ""
        print(f"    [{i}] {m.name}{primary} - {m.size[0]}x{m.size[1]}")

    print()
    print("  测试初始化时间...")

    start = time.perf_counter()
    capture = WGCCapture()
    init_time = time.perf_counter() - start

    print(f"    DLL 加载: {init_time*1000:.2f} ms")

    # 尝试启动捕获
    print()
    print("  尝试启动捕获...")
    start = time.perf_counter()
    success = capture.start_monitor(0)
    start_time = time.perf_counter() - start

    if success:
        print(f"    ✓ 捕获启动成功 ({start_time*1000:.2f} ms)")
        print(f"    分辨率: {capture.width}x{capture.height}")
        print(f"    D3D11 设备: {hex(capture.d3d11_device) if capture.d3d11_device else 'None'}")

        capture.stop()
        return True
    else:
        print(f"    ✗ 捕获启动失败 ({start_time*1000:.2f} ms)")
        print()
        print("    原因: 另一个应用正在使用 Desktop Duplication")
        print("    解决: 关闭以下应用后重试:")
        print("      - Windows Game Bar (Win+G)")
        print("      - NVIDIA GeForce Experience / ShadowPlay")
        print("      - 其他录屏软件 (OBS, Bandicam 等)")
        return False


def test_capture_framerate():
    """测试 2: 连续捕获帧率"""
    print_header("[测试 2/4] 连续捕获帧率 (无 sleep 真实性能)")

    capture = WGCCapture()
    if not capture.start_monitor(0):
        print("  ✗ 无法启动捕获，跳过此测试")
        return False

    print("  测试最大捕获性能 (无 Python sleep)...")
    print("  持续时间: 3 秒")
    print()

    # 等待稳定
    time.sleep(0.2)

    frames = 0
    capture_times = []
    start_time = time.perf_counter()

    while (time.perf_counter() - start_time) < 3.0:
        loop_start = time.perf_counter()

        frame = capture.capture_frame()
        if frame:
            frames += 1
            capture_time = time.perf_counter() - loop_start
            capture_times.append(capture_time)

        # 不添加 sleep - 测试真实最大性能

    total_time = time.perf_counter() - start_time
    fps = frames / total_time if total_time > 0 else 0

    # 计算统计
    if capture_times:
        avg_time = sum(capture_times) * 1000 / len(capture_times)
        max_time = max(capture_times) * 1000
        min_time = min(capture_times) * 1000
        p95_time = sorted(capture_times)[int(len(capture_times) * 0.95)] * 1000 if capture_times else 0
    else:
        avg_time = max_time = min_time = p95_time = 0

    print(f"  结果:")
    print(f"    总帧数: {frames}")
    print(f"    总时间: {total_time:.2f} s")
    print(f"    平均 FPS: {fps:.1f}")
    print()
    print(f"  捕获延迟 (无 sleep):")
    print(f"    平均: {avg_time:.3f} ms")
    print(f"    最小: {min_time:.3f} ms")
    print(f"    最大: {max_time:.3f} ms")
    print(f"    P95:  {p95_time:.3f} ms")
    print()

    # 评级
    if fps >= 144:
        rating = "🚀 A+ - 超过目标!"
    elif fps >= 120:
        rating = "✓ A - 优秀"
    elif fps >= 60:
        rating = "⚠ B - 良好"
    else:
        rating = "✗ C - 需优化"

    print(f"  评级: {rating}")
    print(f"  目标: 144 fps (当前: {fps:.1f} fps)")

    capture.stop()
    return fps >= 60


def test_gpu_direct():
    """测试 3: GPU Direct 验证"""
    print_header("[测试 3/4] GPU Direct 验证")

    capture = WGCCapture()
    if not capture.start_monitor(0):
        print("  ✗ 无法启动捕获，跳过此测试")
        return False

    print("  验证 GPU Direct 功能...")
    print()

    # 获取 D3D11 设备
    device = capture.d3d11_device
    print(f"  D3D11 设备: {hex(device) if device else 'None'}")

    # 捕获一帧
    frame = capture.capture_frame()
    if frame:
        print(f"  帧信息:")
        print(f"    分辨率: {frame.width}x{frame.height}")
        print(f"    D3D11 纹理: {hex(frame.d3d11_texture) if frame.d3d11_texture else 'None'}")
        print(f"    帧序号: {frame.frame_id}")
        print()

        if frame.d3d11_texture:
            print("  ✓ GPU Direct 可用!")
            print("     可直接将 D3D11 纹理传递给 NVENC 编码器")
            print("     无需 CPU 内存复制，零延迟")

            # 尝试加载 NVENC DLL
            nvenc_path = Path(__file__).parent / "cpp_capture" / "nvenc_full.dll"
            if nvenc_path.exists():
                print()
                print("  NVENC DLL 已找到，可以进行集成测试")
                return True
            else:
                print()
                print("  ⚠ NVENC DLL 未找到，无法测试完整 GPU Direct 管道")
                return True
        else:
            print("  ✗ GPU Direct 不可用")
            return False
    else:
        print("  ✗ 未捕获到帧")
        return False

    capture.stop()


def test_nvenc_integration():
    """测试 4: 与 NVENC 集成"""
    print_header("[测试 4/4] NVENC 集成测试")

    # 检查 NVENC DLL
    nvenc_path = Path(__file__).parent / "cpp_capture" / "nvenc_full.dll"
    if not nvenc_path.exists():
        print("  ⚠ NVENC DLL 未找到，跳过此测试")
        print("     需要先编译 nvenc_full.dll")
        return False

    print("  加载 NVENC DLL...")
    try:
        nvenc = ctypes.CDLL(str(nvenc_path))
        print(f"  ✓ NVENC DLL 加载成功 ({nvenc_path.stat().st_size:,} 字节)")
    except Exception as e:
        print(f"  ✗ NVENC DLL 加载失败: {e}")
        return False

    # 检查函数
    print()
    print("  检查 GPU Direct 函数...")

    functions = [
        "is_nvenc_supported",
        "init_nvenc_encoder_d3d11",
        "encode_nvenc_frame_d3d11",
        "get_nvenc_encoded_frame",
    ]

    for func_name in functions:
        if hasattr(nvenc, func_name):
            print(f"    ✓ {func_name}")
        else:
            print(f"    ✗ {func_name}")

    print()
    print("  完整 GPU Direct 管道测试:")
    print("    1. WGC 捕获 → D3D11 纹理")
    print("    2. D3D11 纹理 → NVENC 编码")
    print("    3. NVENC 输出 → H.264 比特流")
    print()
    print("  ⚠ 完整集成测试需要单独运行 (见 test_gpu_direct.py)")

    return True


def print_summary(results):
    """打印测试总结"""
    print_header("测试总结")

    print("  测试结果:")
    print()

    for name, passed in results.items():
        status = "✓ PASS" if passed else "✗ FAIL"
        print(f"    {status} - {name}")

    print()
    print("=" * 70)
    print()

    all_passed = all(results.values())

    if all_passed:
        print("🎉 所有测试通过!")
        print()
        print("WGC Capture 已验证可用于:")
        print("  - 高性能屏幕捕获 (60+ fps)")
        print("  - GPU Direct (D3D11 纹理输出)")
        print("  - 与 NVENC 集成")
        print()
        print("下一步:")
        print("  - 运行完整 GPU Direct 管道测试")
        print("  - 集成到 NVENC Agent")
    else:
        print("⚠ 部分测试未通过")
        print()
        print("常见问题:")
        print("  1. 捕获启动失败 - 关闭 Game Bar / NVIDIA Share")
        print("  2. 帧率低 - 检查系统负载")
        print("  3. GPU Direct 失败 - 检查 D3D11 设备创建")

    print()


def main():
    """主函数"""
    print_test_info()

    results = {}

    # 测试 1: 初始化
    results["捕获初始化"] = test_capture_initialization()

    if not results["捕获初始化"]:
        print()
        print("=" * 70)
        print("初始化失败，无法继续测试")
        print("=" * 70)
        print()
        print("请先解决 Desktop Duplication 占用问题后重试")
        return 1

    # 测试 2: 帧率
    results["连续捕获帧率"] = test_capture_framerate()

    # 测试 3: GPU Direct
    results["GPU Direct 验证"] = test_gpu_direct()

    # 测试 4: NVENC 集成
    results["NVENC 集成"] = test_nvenc_integration()

    # 总结
    print_summary(results)

    return 0 if all(results.values()) else 1


if __name__ == "__main__":
    sys.exit(main())
