@echo off
call "D:\Program Files\Microsoft Visual Studio\2022\Community\VC\Auxiliary\Build\vcvars64.bat"
cd /d J:\ProjectTest\远程探查\mini-remote-desktop\agent-python\cpp_capture

echo ====================================================================
echo Building nvenc_full.dll with CUDA kernel
echo ====================================================================

set NVCC="C:\Program Files\NVIDIA GPU Computing Toolkit\CUDA\v13.0\bin\nvcc.exe"
set CUDA_INC="C:\Program Files\NVIDIA GPU Computing Toolkit\CUDA\v13.0\include"
set CUDA_LIB="C:\Program Files\NVIDIA GPU Computing Toolkit\CUDA\v13.0\lib\x64"

REM 编译 CUDA kernel (.cu -> .obj)
echo [1/3] Compiling CUDA kernel...
%NVCC% -c bgra_to_nv12.cu -o bgra_to_nv12.obj -I"%CUDA_INC%" -I".\nvenc_headers" -Xcompiler "/Zi /MD" -DNOMINMAX

if errorlevel 1 (
    echo FAILED: CUDA kernel compilation
    exit /b 1
)

REM 编译 C++ 代码 (.cpp -> .obj)
echo [2/3] Compiling C++ code...
cl /c /EHsc /std:c++20 /MD /Zi -I".\nvenc_headers" -I"%CUDA_INC%" nvenc_full.cpp

if errorlevel 1 (
    echo FAILED: C++ compilation
    exit /b 1
)

REM 链接生成 DLL
echo [3/3] Linking DLL...
link /DLL /OUT:nvenc_full.dll nvenc_full.obj bgra_to_nv12.obj d3d11.lib dxgi.lib cuda.lib cudart.lib /LIBPATH:"%CUDA_LIB%"

if errorlevel 1 (
    echo FAILED: Linking
    exit /b 1
)

echo ====================================================================
echo SUCCESS! nvenc_full.dll created with CUDA kernel support
echo ====================================================================
dir nvenc_full.dll

