#!/usr/bin/env python3
"""Build wgc_winrt.dll with C++/WinRT support"""

import subprocess
import sys
import os
from pathlib import Path

vs_path = r"D:\Program Files\Microsoft Visual Studio\2022\Community"
vc_tools = r"D:\Program Files\Microsoft Visual Studio\2022\Community\VC\Tools\MSVC\14.44.35207"
winsdk = r"C:\Program Files (x86)\Windows Kits\10"

# 设置环境变量
include_paths = [
    f"{vc_tools}\\include",
    f"{winsdk}\\Include\\10.0.26100.0\\ucrt",
    f"{winsdk}\\Include\\10.0.26100.0\\shared",
    f"{winsdk}\\Include\\10.0.26100.0\\um",
    f"{winsdk}\\Include\\10.0.26100.0\\winrt",
]

lib_paths = [
    f"{vc_tools}\\lib\\x64",
    f"{winsdk}\\Lib\\10.0.26100.0\\ucrt\\x64",
    f"{winsdk}\\Lib\\10.0.26100.0\\um\\x64",
]

env = os.environ.copy()
env["INCLUDE"] = ";".join(include_paths)
env["LIB"] = ";".join(lib_paths)

print("=" * 70)
print("Building wgc_winrt.dll (C++/WinRT)")
print("=" * 70)

# 编译命令
cmd = [
    "cl.exe",
    "/LD",           # DLL
    "/MD",           # Multi-threaded runtime
    "/O2",           # Optimize
    "/EHsc",         # Exceptions
    "/permissive-",  # 严格标准
    "/await",        # Coroutines (C++/WinRT 需要)
    "/ZW",           # C++/WinRT (隐含 /std:c++17)
    "/DWGC_WINRT_EXPORTS",
    "/bigobj",       # 大对象文件
    "wgc_winrt.cpp",
    "/link",
    "d3d11.lib",
    "dxgi.lib",
    "dwmapi.lib",
    "user32.lib",
    "windowsapp.lib",
    "/OUT:wgc_winrt.dll"
]

# 运行 vcvars64.bat 并编译
batch_cmd = f'"{vs_path}\\VC\\Auxiliary\\Build\\vcvars64.bat" && {" ".join(cmd)}'

print(f"Running: {batch_cmd}")
print()

result = subprocess.run(
    f'cmd /c "{batch_cmd}"',
    capture_output=True,
    text=True,
    env=env,
    shell=True
)

print(result.stdout)
if result.stderr:
    print("STDERR:")
    print(result.stderr)

# 检查结果
dll_path = Path("wgc_winrt.dll")
if dll_path.exists():
    print()
    print("=" * 70)
    print(f"SUCCESS: DLL created ({dll_path.stat().st_size:,} bytes)")
    print("=" * 70)
    sys.exit(0)
else:
    print()
    print("=" * 70)
    print("FAILED: DLL not created")
    print("=" * 70)

    # 输出诊断信息
    print()
    print("Diagnostic information:")
    print(f"  VS path exists: {Path(vs_path).exists()}")
    print(f"  SDK path exists: {Path(winsdk).exists()}")
    print(f"  wgc_winrt.cpp exists: {Path('wgc_winrt.cpp').exists()}")

    sys.exit(1)
