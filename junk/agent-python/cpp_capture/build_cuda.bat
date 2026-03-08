@echo off
REM 编译 CUDA Kernel
REM 需要 CUDA Toolkit

set CUDA_PATH=C:\Program Files\NVIDIA GPU Computing Toolkit\CUDA\v13.0

echo Compiling BGRA to NV12 CUDA kernel...

"%CUDA_PATH%\bin\nvcc" ^
    -ptx ^
    -m=64 ^
    -O3 ^
    -Line-info ^
    -arch=sm_75 ^
    bgra_to_nv12.cu ^
    -o bgra_to_nv12.ptx

if %errorlevel% neq 0 (
    echo CUDA compilation failed!
    exit /b 1
)

echo CUDA kernel compiled successfully: bgra_to_nv12.ptx
