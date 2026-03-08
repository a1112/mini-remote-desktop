#!/usr/bin/env python3
"""
NVENC 动态加载编码器简单测试

只测试 NVENC DLL 的基本功能，不依赖捕获设备
"""
import sys
import time
import ctypes
from pathlib import Path

print("=" * 70)
print("NVENC 动态加载编码器简单测试")
print("=" * 70)

# ============================================================================
# 1. 检查 DLL 文件
# ============================================================================

print("\n[1/4] 检查 DLL 文件...")

nvenc_dll_path = Path(__file__).parent / 'cpp_capture' / 'nvenc_d3d12_dynamic.dll'

if not nvenc_dll_path.exists():
    print(f"  ❌ DLL 不存在: {nvenc_dll_path}")
    sys.exit(1)

print(f"  ✅ DLL 文件: {nvenc_dll_path}")
print(f"  ✅ 文件大小: {nvenc_dll_path.stat().st_size} bytes")

# ============================================================================
# 2. 加载 DLL
# ============================================================================

print("\n[2/4] 加载 DLL...")

try:
    nvenc_dll = ctypes.CDLL(str(nvenc_dll_path))
    print(f"  ✅ DLL 加载成功")
except Exception as e:
    print(f"  ❌ DLL 加载失败: {e}")
    sys.exit(1)

# ============================================================================
# 3. 测试导出函数
# ============================================================================

print("\n[3/4] 测试导出函数...")

# 函数列表
functions_to_test = [
    ('is_nvenc_supported', [], ctypes.c_int),
    ('is_cuda_d3d11_interop_supported', [], ctypes.c_int),
    ('init_nvenc_encoder_d3d11', [ctypes.c_void_p, ctypes.c_void_p, ctypes.c_void_p], ctypes.c_void_p),
    ('encode_nvenc_frame_cpu', [ctypes.c_void_p, ctypes.POINTER(ctypes.c_ubyte), ctypes.c_int, ctypes.c_longlong, ctypes.c_int], ctypes.c_int),
    ('encode_nvenc_frame_d3d11', [ctypes.c_void_p, ctypes.c_void_p, ctypes.c_longlong, ctypes.c_int], ctypes.c_int),
    ('get_nvenc_encoded_frame', [ctypes.c_void_p, ctypes.c_void_p], ctypes.c_int),
    ('free_nvenc_encoded_frame', [ctypes.c_void_p], None),
    ('request_nvenc_keyframe', [ctypes.c_void_p], None),
    ('free_nvenc_encoder', [ctypes.c_void_p], None),
    ('get_nvenc_version', [ctypes.c_void_p], None),
]

found_functions = []
for func_name, argtypes, restype in functions_to_test:
    try:
        func = getattr(nvenc_dll, func_name)
        if argtypes:
            func.argtypes = argtypes
        if restype is not None:
            func.restype = restype
        found_functions.append(func_name)
        print(f"  ✅ {func_name}")
    except AttributeError:
        print(f"  ❌ {func_name} (not found)")

print(f"\n  找到 {len(found_functions)}/{len(functions_to_test)} 个函数")

# ============================================================================
# 4. 测试功能
# ============================================================================

print("\n[4/4] 测试功能...")

# 测试 NVENC 支持
nvenc_sup = nvenc_dll.is_nvenc_supported()
print(f"  NVENC 支持: {'✅ 是' if nvenc_sup else '❌ 否'}")

# 测试 CUDA-D3D11 互操作
cuda_sup = nvenc_dll.is_cuda_d3d11_interop_supported()
print(f"  CUDA-D3D11 互操作: {'✅ 是' if cuda_sup else '❌ 否'}")

# 测试版本
class NVENCVersion(ctypes.Structure):
    _fields_ = [("major", ctypes.c_int), ("minor", ctypes.c_int)]

version = NVENCVersion()
nvenc_dll.get_nvenc_version(ctypes.byref(version))
print(f"  NVENC 版本: {version.major}.{version.minor}")

# ============================================================================
# 总结
# ============================================================================

print("\n" + "=" * 70)
print("测试结果")
print("=" * 70)

if nvenc_sup and cuda_sup:
    print("✅ NVENC 动态加载编码器就绪")
    print("\n下一步:")
    print("  1. 当前实现为存根 (stub) 版本")
    print("  2. 需要完整 NVENC API 才能实际编码")
    print("  3. 下载 NVIDIA Video Codec SDK:")
    print("     https://developer.nvidia.com/nvenc-sdk")
    print("  4. 动态加载 NVENC API 函数指针")
    print("  5. 实现实际的编码功能")
else:
    print("⚠️  NVENC 不可用，将使用回退编码器")

print("\n架构说明:")
print("  - 使用 D3D11-CUDA 互操作 (CUDA 11.0+ 支持)")
print("  - 动态加载 nvEncodeAPI64.dll")
print("  - 无需编译时依赖 NVENC SDK")
print("  - 支持运行时回退到软件编码")
