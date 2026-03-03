@echo off
setlocal

REM 设置路径
set CUDA_PATH=C:\Program Files\NVIDIA GPU Computing Toolkit\CUDA\v13.0
set NVCC=%CUDA_PATH%\bin\nvcc.exe

REM 设置工作目录
cd /d J:\ProjectTest\远程探查\mini-remote-desktop\agent-python\cpp_capture

echo Compiling CUDA kernel to PTX...

REM 编译为 PTX
"%NVCC%" -ptx bgra_to_nv12.cu -o bgra_to_nv12.ptx ^
    -I"%CUDA_PATH%\include" ^
    -I".\nvenc_headers" ^
    -DNOMINMAX

if errorlevel 1 (
    echo FAILED: PTX compilation
    exit /b 1
)

echo SUCCESS: bgra_to_nv12.ptx created
dir bgra_to_nv12.ptx

endlocal
