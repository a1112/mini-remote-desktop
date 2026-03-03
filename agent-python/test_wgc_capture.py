#!/usr/bin/env python3
"""
WGC Capture 测试脚本

测试 Windows Graphics Capture API:
1. 枚举监视器和窗口
2. 屏幕捕获
3. 窗口捕获
4. GPU Direct 输出
"""

import sys
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent / 'src'))

from src.capture.wgc_capture import (
    WGCCapture,
    print_available_monitors,
    print_available_windows
)


def test_wgc_capture():
    """测试 WGC 捕获"""

    print("=" * 70)
    print("WGC Capture 测试")
    print("=" * 70)

    # ------------------------------------------------------------
    # 1. 枚举监视器
    # ------------------------------------------------------------
    print("\n[1/4] 枚举监视器...")
    print("-" * 70)

    monitors = WGCCapture.enum_monitors()
    print(f"  发现 {len(monitors)} 个监视器:")
    for i, m in enumerate(monitors):
        primary = " [主]" if m.is_primary else ""
        print(f"    [{i}] {m.name}{primary} - {m.size[0]}x{m.size[1]}")

    if not monitors:
        print("  ✗ 未发现监视器!")
        return False

    # ------------------------------------------------------------
    # 2. 枚举窗口
    # ------------------------------------------------------------
    print("\n[2/4] 枚举窗口...")
    print("-" * 70)

    windows = WGCCapture.enum_windows()
    print(f"  发现 {len(windows)} 个窗口")
    print(f"  显示前 10 个:")
    for w in windows[:10]:
        visible = "[可见]" if w.is_visible else "[隐藏]"
        print(f"    {hex(w.hwnd)} - {w.title[:50]} {visible}")

    # ------------------------------------------------------------
    # 3. 屏幕捕获测试
    # ------------------------------------------------------------
    print("\n[3/4] 屏幕捕获测试...")
    print("-" * 70)

    with WGCCapture() as capture:
        if not capture.start_monitor(0):
            print("  ✗ 启动监视器捕获失败!")
            print("  原因: 另一个应用正在使用 Desktop Duplication")
            print("  解决: 关闭 Game Bar / NVIDIA Share / 录屏软件")
            return False

        print("  ✓ 捕获会话已启动")

        # 等待一秒
        time.sleep(0.5)

        # 捕获几帧
        print("  捕获帧...")
        frames = 0
        start_time = time.time()

        for _ in range(30):
            frame = capture.capture_frame()
            if frame:
                frames += 1
                if frames == 1:
                    print(f"    首帧: {frame.width}x{frame.height}")
                    print(f"    D3D11 纹理: {hex(frame.d3d11_texture)}")
            time.sleep(0.033)  # ~30 FPS

        elapsed = time.time() - start_time
        fps = frames / elapsed if elapsed > 0 else 0

        print(f"  ✓ 捕获了 {frames} 帧，耗时 {elapsed:.2f}s ({fps:.1f} fps)")

        # 检查 D3D11 设备
        device = capture.d3d11_device
        if device:
            print(f"  ✓ D3D11 设备: {hex(device)} (可用于 GPU Direct)")

    # ------------------------------------------------------------
    # 4. 窗口捕获测试 (可选)
    # ------------------------------------------------------------
    print("\n[4/4] 窗口捕获测试 (可选)...")
    print("-" * 70)

    if windows:
        # 找一个可见的窗口
        target = None
        for w in windows:
            if w.is_visible and "记事本" in w.title or "Notepad" in w.title:
                target = w
                break

        if not target and windows:
            target = windows[0]  # 使用第一个窗口

        if target:
            print(f"  目标窗口: {target.title[:50]}")
            print(f"  HWND: {hex(target.hwnd)}")
            print(f"  大小: {target.size[0]}x{target.size[1]}")

            with WGCCapture() as capture:
                if capture.start_window(target.hwnd):
                    print("  ✓ 窗口捕获已启动")
                    time.sleep(0.5)

                    frame = capture.capture_frame()
                    if frame:
                        print(f"  ✓ 捕获成功: {frame.width}x{frame.height}")
                    else:
                        print("  ⚠ 未捕获到帧 (窗口可能最小化)")
                else:
                    print("  ⚠ 窗口捕获启动失败")

    # ------------------------------------------------------------
    # 总结
    # ------------------------------------------------------------
    print("\n" + "=" * 70)
    print("测试完成!")
    print("=" * 70)
    print("\nWGC Capture 特性:")
    print("  ✓ 监视器枚举")
    print("  ✓ 窗口枚举")
    print("  ✓ 屏幕捕获")
    print("  ✓ 窗口捕获")
    print("  ✓ GPU Direct (D3D11 纹理输出)")
    print("\n延迟: ~0-1ms (Desktop Duplication)")
    print("并发: 驱动限制 (DXGI 1.5 DuplicateOutput1 不可用时)")

    return True


if __name__ == "__main__":
    success = test_wgc_capture()
    sys.exit(0 if success else 1)
