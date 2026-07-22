import subprocess
import os

os.chdir(r"J:\ProjectTest\远程探查\mini-remote-desktop\agent-python\cpp_capture")

# 设置 VS 环境
vs_path = r"D:\Program Files\Microsoft Visual Studio\2022\Community\VC\Auxiliary\Build\vcvars64.bat"

# 编译命令
compile_cmd = f'call "{vs_path}" && cl.exe /LD /MD /O2 /EHsc /std:c++17 dxgi_capture.cpp /link d3d11.lib dxgi.lib /OUT:dxgi_capture.dll'

# 执行
result = subprocess.run(
    compile_cmd,
    shell=True,
    capture_output=True,
    text=True,
    timeout=60
)

print("=== STDOUT ===")
print(result.stdout)
print("\n=== STDERR ===")
print(result.stderr)
print(f"\nReturn code: {result.returncode}")

# 检查 DLL
if os.path.exists("dxgi_capture.dll"):
    print("\n✅ DLL 创建成功!")
    size = os.path.getsize("dxgi_capture.dll")
    print(f"   大小: {size} bytes")
else:
    print("\n❌ DLL 未创建")
