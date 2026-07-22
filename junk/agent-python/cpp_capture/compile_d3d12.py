"""
D3D12 混合捕获编译脚本 - Python
"""
import subprocess
import sys
from pathlib import Path

print("=" * 70)
print("D3D12 Hybrid Capture - Python 编译脚本")
print("=" * 70)

# 配置
VS_PATH = r"D:\Program Files\Microsoft Visual Studio\2022\Community"
VCVARS = VS_PATH + r"\VC\Auxiliary\Build\vcvars64.bat"
SRC_FILE = "d3d12_hybrid_capture.cpp"
DLL_NAME = "d3d12_hybrid_capture.dll"

# 检查源文件
if not Path(SRC_FILE).exists():
    print(f"❌ 源文件不存在: {SRC_FILE}")
    sys.exit(1)

print(f"\n[1/3] 检查 Visual Studio...")
if not Path(VCVARS).exists():
    print(f"❌ Visual Studio 不在: {VCVARS}")
    sys.exit(1)
print(f"✅ Visual Studio: {VS_PATH}")

print(f"\n[2/3] 编译 {DLL_NAME}...")

# 构建编译命令
compile_cmd = f'call "{VCVARS}" && cl.exe /LD /MD /O2 /EHsc /std:c++17 {SRC_FILE} /link d3d11.lib dxgi.lib d3d12.lib /OUT:{DLL_NAME}'

# 使用 cmd 运行
result = subprocess.run(
    compile_cmd,
    shell=True,
    capture_output=True,
    text=True
)

print("\n编译器输出:")
if result.stdout:
    for line in result.stdout.split('\n')[-20:]:  # 最后 20 行
        if line.strip():
            print(f"  {line}")

if result.stderr:
    for line in result.stderr.split('\n')[-20:]:
        if line.strip() and ' Initializing' not in line and ' environment' not in line:
            print(f"  {line}")

print(f"\n[3/3] 检查结果...")

if Path(DLL_NAME).exists():
    size = Path(DLL_NAME).stat().st_size
    print(f"✅ 编译成功: {DLL_NAME} ({size} bytes)")
    print("\n" + "=" * 70)
    print("SUCCESS")
    print("=" * 70)
else:
    print(f"❌ 编译失败: DLL 未生成")
    sys.exit(1)
