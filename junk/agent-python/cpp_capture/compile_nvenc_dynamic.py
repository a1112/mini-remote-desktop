"""
编译 NVENC 动态加载版本编译器

使用 CUDA 13.0，不需要 NVENC SDK
"""
import subprocess
import sys
import os
from pathlib import Path

print("=" * 70)
print("NVENC 动态加载版本编译脚本")
print("=" * 70)

# 配置
CUDA_PATH = r"C:\Program Files\NVIDIA GPU Computing Toolkit\CUDA\v13.0"
CUDA_LIB = os.path.join(CUDA_PATH, "lib", "x64")
CUDA_INCLUDE = os.path.join(CUDA_PATH, "include")

VS_PATH = r"D:\Program Files\Microsoft Visual Studio\2022\Community"
VCVARS = os.path.join(VS_PATH, "VC", "Auxiliary", "Build", "vcvars64.bat")

# 源文件
SRC_FILE = "nvenc_d3d12_dynamic.cpp"
DLL_NAME = "nvenc_d3d12_dynamic.dll"

print(f"\n[1/4] 检查环境...")

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

# 检查 VS
if os.path.exists(VCVARS):
    print(f"  ✅ Visual Studio: {VS_PATH}")
else:
    print(f"  ❌ Visual Studio 不在: {VCVARS}")
    print(f"    尝试修改 VS_PATH")
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
print(f"\n[2/4] 编译 {DLL_NAME}...")

include_dirs = [
    f"/I\"{CUDA_INCLUDE}\"",
]

lib_dirs = [
    f"/LIBPATH:\"{CUDA_LIB}\"",
]

libs = [
    "cuda.lib",
    "cudart.lib",
    "d3d12.lib",
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
    for line in result.stdout.split('\n')[-30:]:
        if line.strip():
            print(f"  {line}")

if result.stderr:
    for line in result.stderr.split('\n')[-30:]:
        if line.strip() and 'Initializing' not in line.lower() and 'environment' not in line.lower():
            print(f"  {line}")

# 检查结果
print(f"\n[3/4] 检查结果...")

if os.path.exists(DLL_NAME):
    size = os.path.getsize(DLL_NAME)
    print(f"  ✅ 编译成功: {DLL_NAME} ({size} bytes)")

    # 测试加载
    try:
        import ctypes
        test_dll = ctypes.CDLL(DLL_NAME)
        print(f"  ✅ DLL 可加载")
    except Exception as e:
        print(f"  ⚠️  DLL 加载失败: {e}")
else:
    print(f"  ❌ 编译失败")
    sys.exit(1)

print(f"\n[4/4] 完成...")
print("=" * 70)
print(f"✅ {DLL_NAME} 已就绪")

# 测试功能
import ctypes
import os

# 添加 CUDA bin 目录到 PATH
cuda_bin = os.path.join(CUDA_PATH, "bin")
os.environ['PATH'] = cuda_bin + os.pathsep + os.environ.get('PATH', '')

print(f"\n导出函数测试:")
dll_path = os.path.abspath(DLL_NAME)
print(f"  加载: {dll_path}")
dll = ctypes.CDLL(dll_path)

# 测试函数
dll.is_nvenc_supported.argtypes = []
dll.is_nvenc_supported.restype = ctypes.c_int

dll.is_cuda_d3d11_interop_supported.argtypes = []
dll.is_cuda_d3d11_interop_supported.restype = ctypes.c_int

nvenc_sup = dll.is_nvenc_supported()
cuda_sup = dll.is_cuda_d3d11_interop_supported()

print(f"  NVENC 支持: {'✅' if nvenc_sup else '❌'}")
print(f"  CUDA-D3D11 互操作: {'✅' if cuda_sup else '❌'}")
print(f"  编码器状态: {'✅ 就绪 (动态加载模式)' if nvenc_sup or cuda_sup else '❌ 需要检查驱动'}")
