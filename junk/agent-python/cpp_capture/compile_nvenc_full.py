"""
编译完整 NVENC 编译器

使用 CUDA 13.0 和 NVENC SDK 13.0
"""
import subprocess
import sys
import os
from pathlib import Path

print("=" * 70)
print("完整 NVENC 编译器编译脚本")
print("=" * 70)

# 配置
CUDA_PATH = r"C:\Program Files\NVIDIA GPU Computing Toolkit\CUDA\v13.0"
CUDA_LIB = os.path.join(CUDA_PATH, "lib", "x64")
CUDA_INCLUDE = os.path.join(CUDA_PATH, "include")

# NVENC SDK 路径
NVENC_SDK_PATH = r"J:\ProjectTest\远程探查\mini-remote-desktop\tools\Video_Codec_Interface_13.0.37\Interface"

VS_PATH = r"D:\Program Files\Microsoft Visual Studio\2022\Community"
VCVARS = os.path.join(VS_PATH, "VC", "Auxiliary", "Build", "vcvars64.bat")

# 源文件
SRC_FILE = "nvenc_full.cpp"
DLL_NAME = "nvenc_full.dll"

print(f"\n[1/5] 检查环境...")

# 检查 CUDA
if os.path.exists(CUDA_INCLUDE):
    print(f"  ✅ CUDA Include: {CUDA_INCLUDE}")
else:
    print(f"  ❌ CUDA Include 不存在: {CUDA_INCLUDE}")
    sys.exit(1)

if os.path.exists(CUDA_LIB):
    print(f"  ✅ CUDA Lib: {CUDA_LIB}")
else:
    print(f"  ❌ CUDA Lib 不存在: {CUDA_LIB}")
    sys.exit(1)

# 检查 NVENC SDK
if os.path.exists(NVENC_SDK_PATH):
    print(f"  ✅ NVENC SDK: {NVENC_SDK_PATH}")
else:
    print(f"  ❌ NVENC SDK 不存在: {NVENC_SDK_PATH}")
    sys.exit(1)

# 检查 VS
if os.path.exists(VCVARS):
    print(f"  ✅ Visual Studio: {VS_PATH}")
else:
    print(f"  ❌ Visual Studio 不在: {VCVARS}")
    sys.exit(1)

# 检查源文件
if not os.path.exists(SRC_FILE):
    print(f"  ❌ 源文件不存在: {SRC_FILE}")
    sys.exit(1)

print(f"  ✅ 源文件: {SRC_FILE}")

# 检查 nvEncodeAPI64.dll
if os.path.exists(r"C:\Windows\System32\nvEncodeAPI64.dll"):
    print(f"  ✅ nvEncodeAPI64.dll: 存在")
else:
    print(f"  ⚠️  nvEncodeAPI64.dll: 不存在")

# 编译选项
print(f"\n[2/5] 编译 {DLL_NAME}...")

include_dirs = [
    f"/I\"{CUDA_INCLUDE}\"",
    f"/I\"{NVENC_SDK_PATH}\"",
]

lib_dirs = [
    f"/LIBPATH:\"{CUDA_LIB}\"",
]

libs = [
    "cuda.lib",
    "cudart.lib",
    "d3d11.lib",
    "dxgi.lib",
]

compile_flags = [
    "/LD",           # DLL
    "/MD",           # Multi-threaded DLL
    "/O2",           # 优化
    "/EHsc",         # 异常处理
    "/std:c++17",    # C++17
    "/DNVENC_ENCODER_EXPORTS",
]

compile_cmd = f'call "{VCVARS}" && cl.exe {" ".join(include_dirs)} {" ".join(compile_flags)} {SRC_FILE} /link {" ".join(lib_dirs)} {" ".join(libs)} /OUT:{DLL_NAME}'

print(f"  命令: {compile_cmd[:200]}...")

result = subprocess.run(
    compile_cmd,
    shell=True,
    capture_output=True,
    text=True
)

print("\n编译器输出:")
if result.stdout:
    for line in result.stdout.split('\n')[-40:]:
        if line.strip():
            print(f"  {line}")

if result.stderr:
    for line in result.stderr.split('\n')[-40:]:
        if line.strip() and 'Initializing' not in line.lower() and 'environment' not in line.lower():
            print(f"  {line}")

# 检查结果
print(f"\n[3/5] 检查结果...")

if os.path.exists(DLL_NAME):
    size = os.path.getsize(DLL_NAME)
    print(f"  ✅ 编译成功: {DLL_NAME} ({size} bytes)")
else:
    print(f"  ❌ 编译失败")
    sys.exit(1)

# 测试功能
print(f"\n[4/5] 测试导出函数...")

import ctypes

# 添加 CUDA bin 目录到 PATH
cuda_bin = os.path.join(CUDA_PATH, "bin")
os.environ['PATH'] = cuda_bin + os.pathsep + os.environ.get('PATH', '')

dll_path = os.path.abspath(DLL_NAME)
print(f"  加载: {dll_path}")

try:
    dll = ctypes.CDLL(dll_path)

    # 测试函数
    functions = [
        'is_nvenc_supported',
        'is_cuda_d3d11_interop_supported',
        'init_nvenc_encoder_d3d11',
        'encode_nvenc_frame_cpu',
        'encode_nvenc_frame_d3d11',
        'get_nvenc_encoded_frame',
        'free_nvenc_encoder',
    ]

    for func_name in functions:
        try:
            func = getattr(dll, func_name)
            print(f"    ✅ {func_name}")
        except AttributeError:
            print(f"    ❌ {func_name} (not found)")

except Exception as e:
    print(f"  ⚠️  DLL 加载失败: {e}")

# 测试版本
print(f"\n[5/5] 测试 NVENC 版本...")

try:
    class NVENCVersion(ctypes.Structure):
        _fields_ = [("major", ctypes.c_int), ("minor", ctypes.c_int)]

    dll.get_nvenc_version.argtypes = [ctypes.POINTER(NVENCVersion)]
    dll.get_nvenc_version.restype = None

    version = NVENCVersion()
    dll.get_nvenc_version(ctypes.byref(version))
    print(f"  NVENC 版本: {version.major}.{version.minor}")

    dll.is_nvenc_supported.argtypes = []
    dll.is_nvenc_supported.restype = ctypes.c_int
    nvenc_sup = dll.is_nvenc_supported()
    print(f"  NVENC 支持: {'✅ 是' if nvenc_sup else '❌ 否'}")

    dll.is_cuda_d3d11_interop_supported.argtypes = []
    dll.is_cuda_d3d11_interop_supported.restype = ctypes.c_int
    cuda_sup = dll.is_cuda_d3d11_interop_supported()
    print(f"  CUDA-D3D11 互操作: {'✅ 是' if cuda_sup else '❌ 否'}")

except Exception as e:
    print(f"  ⚠️  测试失败: {e}")

print(f"\n完成...")
print("=" * 70)
print(f"✅ {DLL_NAME} 已就绪")
