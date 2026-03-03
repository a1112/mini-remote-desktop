#!/usr/bin/env python3
"""
NVENC 编码器 API 测试

测试 NVENC DLL 的导出函数和基本功能
不需要真实的 D3D11 设备
"""
import sys
import ctypes
from pathlib import Path

print("=" * 70)
print("NVENC 编码器 API 测试")
print("=" * 70)

# ============================================================================
# 1. 测试动态加载版本
# ============================================================================

print("\n[1/2] 测试动态加载版本...")

dynamic_dll_path = Path(__file__).parent / 'cpp_capture' / 'nvenc_d3d12_dynamic.dll'

if dynamic_dll_path.exists():
    try:
        dynamic_dll = ctypes.CDLL(str(dynamic_dll_path))

        # 设置函数签名
        dynamic_dll.is_nvenc_supported.argtypes = []
        dynamic_dll.is_nvenc_supported.restype = ctypes.c_int
        dynamic_dll.is_cuda_d3d11_interop_supported.argtypes = []
        dynamic_dll.is_cuda_d3d11_interop_supported.restype = ctypes.c_int

        class NVENCVersion(ctypes.Structure):
            _fields_ = [("major", ctypes.c_int), ("minor", ctypes.c_int)]
        dynamic_dll.get_nvenc_version.argtypes = [ctypes.POINTER(NVENCVersion)]
        dynamic_dll.get_nvenc_version.restype = None

        version = NVENCVersion()
        dynamic_dll.get_nvenc_version(ctypes.byref(version))
        print(f"  ✅ nvenc_d3d12_dynamic.dll 加载成功")
        print(f"     版本: {version.major}.{version.minor}")
        print(f"     NVENC 支持: {'✅' if dynamic_dll.is_nvenc_supported() else '❌'}")
        print(f"     CUDA-D3D11: {'✅' if dynamic_dll.is_cuda_d3d11_interop_supported() else '❌'}")
    except Exception as e:
        print(f"  ⚠️  nvenc_d3d12_dynamic.dll 测试失败: {e}")
else:
    print(f"  ⚠️  nvenc_d3d12_dynamic.dll 不存在")

# ============================================================================
# 2. 测试完整 SDK 版本
# ============================================================================

print("\n[2/2] 测试完整 SDK 版本...")

full_dll_path = Path(__file__).parent / 'cpp_capture' / 'nvenc_full.dll'

if full_dll_path.exists():
    try:
        full_dll = ctypes.CDLL(str(full_dll_path))

        # 设置函数签名
        full_dll.is_nvenc_supported.argtypes = []
        full_dll.is_nvenc_supported.restype = ctypes.c_int
        full_dll.is_cuda_d3d11_interop_supported.argtypes = []
        full_dll.is_cuda_d3d11_interop_supported.restype = ctypes.c_int

        class NVENCVersion(ctypes.Structure):
            _fields_ = [("major", ctypes.c_int), ("minor", ctypes.c_int)]
        full_dll.get_nvenc_version.argtypes = [ctypes.POINTER(NVENCVersion)]
        full_dll.get_nvenc_version.restype = None

        version = NVENCVersion()
        full_dll.get_nvenc_version(ctypes.byref(version))
        print(f"  ✅ nvenc_full.dll 加载成功")
        print(f"     版本: {version.major}.{version.minor}")
        print(f"     NVENC 支持: {'✅' if full_dll.is_nvenc_supported() else '❌'}")
        print(f"     CUDA-D3D11: {'✅' if full_dll.is_cuda_d3d11_interop_supported() else '❌'}")
        print(f"     文件大小: {full_dll_path.stat().st_size} bytes")

        # 检查所有导出函数
        functions = [
            'is_nvenc_supported',
            'is_cuda_d3d11_interop_supported',
            'init_nvenc_encoder_d3d11',
            'encode_nvenc_frame_cpu',
            'encode_nvenc_frame_d3d11',
            'get_nvenc_encoded_frame',
            'free_nvenc_encoded_frame',
            'request_nvenc_keyframe',
            'free_nvenc_encoder',
            'get_nvenc_version',
        ]
        print(f"\n  导出函数检查:")
        all_found = True
        for func_name in functions:
            try:
                getattr(full_dll, func_name)
                print(f"    ✅ {func_name}")
            except AttributeError:
                print(f"    ❌ {func_name} (缺失)")
                all_found = False

        if all_found:
            print(f"\n  ✅ 所有 {len(functions)} 个函数都已导出")

    except Exception as e:
        print(f"  ⚠️  nvenc_full.dll 测试失败: {e}")
else:
    print(f"  ❌ nvenc_full.dll 不存在")

# ============================================================================
# 总结
# ============================================================================

print("\n" + "=" * 70)
print("测试总结")
print("=" * 70)

print("\n可用的 NVENC 编码器 DLL:")

if dynamic_dll_path.exists():
    size = dynamic_dll_path.stat().st_size
    print(f"  1. nvenc_d3d12_dynamic.dll ({size} bytes)")
    print(f"     - 动态加载版本，无需 SDK")
    print(f"     - 使用 CUDA-D3D11 互操作")
    print(f"     - 存根实现，需要完整 SDK 才能实际编码")

if full_dll_path.exists():
    size = full_dll_path.stat().st_size
    print(f"  2. nvenc_full.dll ({size} bytes)")
    print(f"     - 完整 SDK 版本 (NVENC SDK 13.0)")
    print(f"     - 完整 NVENC API 支持")
    print(f"     - 需要真实 D3D11 设备才能初始化")

print("\n下一步:")
print("  1. 使用混合捕获 DLL 获取真实 D3D11 设备")
print("  2. 初始化 NVENC 编码器")
print("  3. 测试完整的编码流水线")

print("\n文件清单:")
print(f"  cpp_capture/nvenc_d3d12_dynamic.dll  - 动态加载版本")
print(f"  cpp_capture/nvenc_full.dll           - 完整 SDK 版本")
print(f"  cpp_capture/d3d12_hybrid_capture.dll - 混合捕获")
