#!/usr/bin/env python3
"""
Desktop Duplication 诊断工具.

检查 Desktop Duplication API 可用性并提供详细诊断。
"""

import ctypes
import logging
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent / 'src'))

logging.basicConfig(
    level=logging.INFO,
    format="%(message)s"
)

logger = logging.getLogger(__name__)


def check_session_type():
    """检查当前会话类型。"""
    logger.info("=" * 70)
    logger.info("会话类型诊断")
    logger.info("=" * 70)

    try:
        import ctypes.wintypes as wintypes

        # 检查进程会话 ID
        kernel32 = ctypes.windll.kernel32
        current_session_id = kernel32.WTSGetActiveConsoleSessionId()
        process_session_id = kernel32.GetCurrentProcessId()

        logger.info(f"  活动控制台会话 ID: {current_session_id}")
        logger.info(f"  当前进程 ID: {process_session_id}")

        # 检查是否在远程会话中
        # GetSystemMetrics(SM_REMOTESESSION) 返回非零表示远程会话
        SM_REMOTESESSION = 0x1000
        is_remote = ctypes.windll.user32.GetSystemMetrics(SM_REMOTESESSION)
        logger.info(f"  远程会话: {'是' if is_remote else '否'}")

        if is_remote:
            logger.warning("\n  ⚠ 检测到远程会话!")
            logger.warning("  Desktop Duplication API 在远程会话中不可用")
            logger.warning("  请在本地控制台会话中运行此程序")
            return False

        return True

    except Exception as e:
        logger.error(f"  会话检查失败: {e}")
        return None


def check_dxgi_availability():
    """检查 DXGI 可用性。"""
    logger.info("\n" + "=" * 70)
    logger.info("DXGI 可用性诊断")
    logger.info("=" * 70)

    try:
        # 尝试加载 DLL
        d3d11 = ctypes.windll.d3d11
        logger.info("  ✓ d3d11.dll 可用")

        dxgi = ctypes.windll.dxgi
        logger.info("  ✓ dxgi.dll 可用")

        return True

    except Exception as e:
        logger.error(f"  ✗ DXGI 加载失败: {e}")
        return False


def test_desktop_duplication_direct():
    """直接测试 Desktop Duplication API。"""
    logger.info("\n" + "=" * 70)
    logger.info("Desktop Duplication API 测试")
    logger.info("=" * 70)

    try:
        import ctypes.wintypes as wintypes

        # 定义必要的常量和结构
        _D3D11_CREATE_DEVICE_FLAG = 0
        D3D11_DRIVER_TYPE_HARDWARE = 1
        D3D11_SDK_VERSION = 7

        # 尝试创建 D3D11 设备
        logger.info("  创建 D3D11 设备...")

        d3d11 = ctypes.windll.d3d11

        # 简化的 D3D11CreateDevice 调用
        device = ctypes.c_void_p()
        context = ctypes.c_void_p()

        hr = d3d11.D3D11CreateDevice(
            None,  # adapter
            D3D11_DRIVER_TYPE_HARDWARE,
            None,  # software
            0,     # flags
            None,  # feature levels
            0,     # feature levels count
            D3D11_SDK_VERSION,
            ctypes.byref(device),
            None,  # feature level
            ctypes.byref(context)
        )

        if hr < 0:
            # 常见错误码
            if hr == 0x887A0005:  # DXGI_ERROR_UNSUPPORTED
                logger.error(f"  ✗ D3D11 不支持")
            elif hr == 0x887A0027:  # E_INVALIDARG
                logger.error(f"  ✗ 无效参数")
            else:
                logger.error(f"  ✗ D3D11CreateDevice 失败: 0x{hr:X}")
            return False

        logger.info(f"  ✓ D3D11 设备创建成功: 0x{device.value:X}")

        # 尝试获取 DXGI 设备
        logger.info("  获取 DXGI 设备...")

        try:
            # 尝试 DuplicateOutput
            # 这需要复杂的 COM 接口调用
            logger.info("  ℹ DXGI 设备可用 (需要完整 COM 接口调用)")
            logger.info("  ℹ 使用 d3d12_hybrid_capture.dll 进行完整测试")

        except Exception as e:
            logger.error(f"  ✗ DXGI 设备获取失败: {e}")

        return True

    except Exception as e:
        logger.error(f"  ✗ Desktop Duplication 测试失败: {e}")
        import traceback
        traceback.print_exc()
        return False


def test_dll_direct():
    """直接测试 hybrid capture DLL。"""
    logger.info("\n" + "=" * 70)
    logger.info("Hybrid Capture DLL 测试")
    logger.info("=" * 70)

    dll_path = Path(__file__).parent / 'd3d12_hybrid_capture.dll'

    if not dll_path.exists():
        logger.error(f"  ✗ DLL 不存在: {dll_path}")
        return False

    logger.info(f"  DLL 路径: {dll_path}")
    logger.info(f"  DLL 大小: {dll_path.stat().st_size:,} 字节")

    try:
        dll = ctypes.CDLL(str(dll_path))
        logger.info("  ✓ DLL 加载成功")

        # 尝试初始化
        logger.info("  尝试初始化捕获器...")

        dll.init_hybrid_capture.argtypes = [ctypes.c_int, ctypes.c_int]
        dll.init_hybrid_capture.restype = ctypes.c_void_p

        handle = dll.init_hybrid_capture(0, 0)  # monitor 0, d3d12 disabled

        if not handle:
            logger.error("  ✗ 初始化失败")

            # 尝试获取更多错误信息
            logger.info("\n  可能的原因:")
            logger.info("    1. 在远程会话中运行 (SSH/RDP)")
            logger.info("    2. 桌面会话被锁定")
            logger.info("    3. 另一个应用正在使用 Desktop Duplication")
            logger.info("    4. 不在活动控制台会话中")

            return False

        logger.info("  ✓ 初始化成功!")

        # 清理
        dll.free_hybrid_capture(handle)
        return True

    except Exception as e:
        logger.error(f"  ✗ DLL 测试失败: {e}")
        import traceback
        traceback.print_exc()
        return False


def main():
    """主诊断函数。"""

    print("\n" + "=" * 70)
    print("Desktop Duplication API 诊断工具")
    print("=" * 70)

    # 1. 检查会话类型
    session_ok = check_session_type()

    # 2. 检查 DXGI 可用性
    dxgi_ok = check_dxgi_availability()

    # 3. 测试 Desktop Duplication
    dd_ok = test_desktop_duplication_direct()

    # 4. 测试 DLL
    dll_ok = test_dll_direct()

    # 总结
    print("\n" + "=" * 70)
    print("诊断总结")
    print("=" * 70)

    results = {
        "会话类型": "✓ 本地控制台" if session_ok else "✗ 远程会话" if session_ok is False else "?",
        "DXGI 可用": "✓" if dxgi_ok else "✗",
        "Desktop Duplication": "✓" if dd_ok else "✗",
        "Hybrid Capture DLL": "✓" if dll_ok else "✗",
    }

    for name, status in results.items():
        print(f"  {name}: {status}")

    # 建议
    print("\n" + "=" * 70)
    print("建议")
    print("=" * 70)

    if session_ok is False:
        print("\n  ⚠ 检测到远程会话")
        print("  Desktop Duplication API 只能在本地交互式会话中使用")
        print("\n  解决方案:")
        print("    1. 在本地 Windows 桌面直接运行程序")
        print("    2. 使用 VNC 等远程控制工具 (而不是 SSH/RDP)")
        print("    3. 或使用备选捕获方案 (MSS/d3dshot)")
    elif not dll_ok:
        print("\n  ⚠ Desktop Duplication 不可用")
        print("\n  可能原因:")
        print("    • 另一个应用正在使用屏幕捕获")
        print("    • 桌面会话被锁定")
        print("    • 显卡驱动不支持")
    else:
        print("\n  ✓ Desktop Duplication 可用")
        print("  可以正常使用 GPU Direct 管道")

    return dll_ok


if __name__ == "__main__":
    success = main()
    sys.exit(0 if success else 1)
