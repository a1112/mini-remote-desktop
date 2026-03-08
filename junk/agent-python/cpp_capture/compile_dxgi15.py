#!/usr/bin/env python3
"""
Compile d3d12_hybrid_capture.dll with DXGI 1.5 support.
"""

import subprocess
import os
import sys
from pathlib import Path

# Setup VS environment
vs_path = r"D:\Program Files\Microsoft Visual Studio\2022\Community"
vc_tools = r"D:\Program Files\Microsoft Visual Studio\2022\Community\VC\Tools\MSVC\14.44.35207"
winsdk = r"C:\Program Files (x86)\Windows Kits\10"

env = {
    "VSINSTALLDIR": vs_path,
    "VCToolsInstallDir": vc_tools,
    "WindowsSdkDir": winsdk,
    "INCLUDE": f"{vc_tools}\\include;{winsdk}\\Include\\10.0.26100.0\\ucrt;{winsdk}\\Include\\10.0.26100.0\\shared;{winsdk}\\Include\\10.0.26100.0\\um",
    "LIB": f"{vc_tools}\\lib\\x64;{winsdk}\\Lib\\10.0.26100.0\\ucrt\\x64;{winsdk}\\Lib\\10.0.26100.0\\um\\x64",
    "PATH": f"{vc_tools}\\bin\\Hostx64\\x64;{os.getenv('PATH', '')}",
}

print("=" * 70)
print("Compiling d3d12_hybrid_capture.dll with DXGI 1.5 support")
print("=" * 70)

# First, call vcvars64.bat to set up the environment properly
vcvars = f'"{vs_path}\\VC\\Auxiliary\\Build\\vcvars64.bat"'

# Build the command
cmd = [
    "cl.exe",
    "/LD",           # Create DLL
    "/MD",           # Multi-threaded runtime DLL
    "/O2",           # Maximize speed
    "/EHsc",         # Exception handling
    "/DD3D12_HYBRID_CAPTURE_EXPORTS",
    "d3d12_hybrid_capture.cpp",
    "/link",
    "d3d11.lib",
    "d3d12.lib",
    "dxgi.lib",
    "/OUT:d3d12_hybrid_capture.dll"
]

print(f"Running: {' '.join(cmd)}")

# Use subprocess with the vcvars64.bat command
result = subprocess.run(
    f'cmd /c "{vcvars} >nul 2>&1 && {" ".join(cmd)}"',
    shell=True,
    capture_output=True,
    text=True,
    env={**os.environ, **env}
)

print(result.stdout)
if result.stderr:
    print(result.stderr)

# Check if DLL was created
dll_path = Path("d3d12_hybrid_capture.dll")
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
