@echo off
REM =============================================================================
REM GPU Direct Capture DLLs 构建脚本
REM =============================================================================
REM
REM 依赖要求:
REM   1. Visual Studio 2019/2022 with C++ 开发负载
REM   2. CUDA Toolkit 12.x (https://developer.nvidia.com/cuda-downloads)
REM   3. NVENC SDK 13.0 (Video_Codec_SDK)
REM   4. CMake 3.15+
REM
REM =============================================================================

setlocal EnableDelayedExpansion

echo ======================================================================
echo GPU Direct Capture DLLs Build Script
echo ======================================================================
echo.

REM 检测 Visual Studio
where cl >nul 2>&1
if %errorlevel% neq 0 (
    echo [ERROR] Visual Studio C++ 编译器未找到!
    echo 请运行此脚本从 "Developer Command Prompt for VS" 或
    echo 使用 "x64 Native Tools Command Prompt for VS"
    echo.
    pause
    exit /b 1
)

REM 设置路径 (根据实际安装位置调整)
set NVENC_SDK_PATH=J:\ProjectTest\远程探查\mini-remote-desktop\tools\Video_Codec_Interface_13.0.37
set CUDA_PATH=C:\Program Files\NVIDIA GPU Computing Toolkit\CUDA\v12.6

echo [INFO] NVENC SDK: %NVENC_SDK_PATH%
echo [INFO] CUDA:       %CUDA_PATH%
echo.

REM 检查 CUDA
if not exist "%CUDA_PATH%\include\cuda.h" (
    echo [WARNING] CUDA 头文件未找到: %CUDA_PATH%\include\cuda.h
    echo 请安装 CUDA Toolkit 或调整 CUDA_PATH
    echo.
)

REM 检查 NVENC SDK
if not exist "%NVENC_SDK_PATH%\Interface\nvEncodeAPI.h" (
    echo [WARNING] NVENC SDK 头文件未找到
    echo 请确保 NVENC SDK 路径正确
    echo.
)

REM 创建构建目录
if not exist build mkdir build
cd build

echo [1/4] 配置 CMake...
cmake .. -G "Ninja" ^
    -DCMAKE_BUILD_TYPE=Release ^
    -DNVENC_SDK_PATH="%NVENC_SDK_PATH%/Interface" ^
    -DCUDA_PATH="%CUDA_PATH%"

if %errorlevel% neq 0 (
    echo.
    echo [ERROR] CMake 配置失败!
    echo 如果 Ninja 未安装，尝试使用 Visual Studio 生成器:
    echo   cmake .. -G "Visual Studio 17 2022" -A x64 ...
    echo.
    cd ..
    pause
    exit /b 1
)

echo.
echo [2/4] 编译 DLLs...
cmake --build . --config Release --parallel

if %errorlevel% neq 0 (
    echo.
    echo [ERROR] 编译失败!
    cd ..
    pause
    exit /b 1
)

echo.
echo [3/4] 检查输出...
dir /B lib\*.dll 2>nul
if %errorlevel% neq 0 (
    echo [WARNING] 未找到 DLL 文件
)

echo.
echo [4/4] 复制到 Python 项目...
copy /Y lib\*.dll ..\*.dll >nul 2>&1

echo.
echo ======================================================================
echo 构建完成!
echo ======================================================================
echo.
echo 输出文件:
echo   - build/lib/d3d12_hybrid_capture.dll
echo   - build/lib/nvenc_full.dll
echo   - build/lib/dxgi_capture.dll
echo.
echo 已复制到: %cd%\..\
echo.

cd ..
pause
