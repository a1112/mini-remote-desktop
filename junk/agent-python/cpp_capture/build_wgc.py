#!/usr/bin/env python3
"""Build wgc_capture.dll"""

import subprocess
import sys
import os
from pathlib import Path

vs_path = r"D:\Program Files\Microsoft Visual Studio\2022\Community"
vc_tools = r"D:\Program Files\Microsoft Visual Studio\2022\Community\VC\Tools\MSVC\14.44.35207"
winsdk = r"C:\Program Files (x86)\Windows Kits\10"

env = {
    "INCLUDE": (
        f"{vc_tools}\\include;"
        f"{winsdk}\\Include\\10.0.26100.0\\ucrt;"
        f"{winsdk}\\Include\\10.0.26100.0\\shared;"
        f"{winsdk}\\Include\\10.0.26100.0\\um;"
        f"{winsdk}\\Include\\10.0.26100.0\\winrt"
    ),
    "LIB": (
        f"{vc_tools}\\lib\\x64;"
        f"{winsdk}\\Lib\\10.0.26100.0\\ucrt\\x64;"
        f"{winsdk}\\Lib\\10.0.26100.0\\um\\x64"
    ),
    "PATH": f"{vc_tools}\\bin\\Hostx64\\x64;{os.getenv('PATH', '')}",
}

print("=" * 70)
print("Building wgc_capture.dll")
print("=" * 70)

cmd = [
    "cl.exe",
    "/LD",           # DLL
    "/MD",           # Multi-threaded runtime
    "/O2",           # Optimize
    "/EHsc",         # Exceptions
    "/std:c++17",    # C++17
    "/DWGC_EXPORTS",
    "wgc_simple.cpp",
    "/link",
    "d3d11.lib",
    "dxgi.lib",
    "dwmapi.lib",
    "user32.lib",
    "/OUT:wgc_capture.dll"
]

result = subprocess.run(
    f'cmd /c ""{vs_path}\\VC\\Auxiliary\\Build\\vcvars64.bat" >nul 2>&1 && {" ".join(cmd)}"',
    capture_output=True,
    text=True,
    env={**os.environ, **env},
    shell=True
)

print(result.stdout)
if result.stderr:
    print(result.stderr)

dll_path = Path("wgc_capture.dll")
if dll_path.exists():
    print("\n" + "=" * 70)
    print(f"SUCCESS: DLL created ({dll_path.stat().st_size:,} bytes)")
    print("=" * 70)
    sys.exit(0)
else:
    print("\n" + "=" * 70)
    print("FAILED: DLL not created")
    print("=" * 70)
    sys.exit(1)
