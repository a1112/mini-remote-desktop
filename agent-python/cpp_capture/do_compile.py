#!/usr/bin/env python3
import subprocess
import sys
import os

vs_path = r"D:\Program Files\Microsoft Visual Studio\2022\Community"
vcvars = os.path.join(vs_path, r"VC\Auxiliary\Build\vcvars64.bat")

cmd = f'"{vcvars}" && cl.exe /LD /MD /O2 /EHsc /DD3D12_HYBRID_CAPTURE_EXPORTS d3d12_hybrid_capture.cpp /link d3d11.lib d3d12.lib dxgi.lib /OUT:d3d12_hybrid_capture.dll'

print("Running compilation...")
print(f"Command: {cmd}")
print("-" * 60)

result = subprocess.run(
    f'cmd /c "{cmd}"',
    capture_output=True,
    text=True,
    shell=True
)

print("STDOUT:")
print(result.stdout)
if result.stderr:
    print("\nSTDERR:")
    print(result.stderr)

print(f"\nReturn code: {result.returncode}")

# Check if DLL was created
if os.path.exists("d3d12_hybrid_capture.dll"):
    size = os.path.getsize("d3d12_hybrid_capture.dll")
    print(f"\n*** SUCCESS: DLL created ({size:,} bytes) ***")
else:
    print("\n*** FAILED: DLL not created ***")

sys.exit(result.returncode)
