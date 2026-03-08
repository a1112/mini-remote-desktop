@echo off
setlocal EnableDelayedExpansion

REM Call VS environment
call "D:\Program Files\Microsoft Visual Studio\2022\Community\VC\Auxiliary\Build\vcvars64.bat"

REM Set paths
set NVENC_SDK=J:\ProjectTest\远程探查\mini-remote-desktop\tools\Video_Codec_Interface_13.0.37\Interface
set CUDA_PATH=C:\Program Files\NVIDIA GPU Computing Toolkit\CUDA\v12.6

echo ====================================================================
echo GPU Direct DLL Build
echo ====================================================================
echo.
echo NVENC SDK: %NVENC_SDK%
echo CUDA:      %CUDA_PATH%
echo.

REM Create and enter build directory
if not exist build mkdir build
cd build

echo [1/2] Configuring CMake...
cmake .. -G "Visual Studio 17 2022" -A x64 ^
    -DCMAKE_BUILD_TYPE=Release ^
    -DNVENC_SDK_PATH="%NVENC_SDK%" ^
    -DCUDA_PATH="%CUDA_PATH%"

if %errorlevel% neq 0 (
    echo CMake configuration failed
    cd ..
    pause
    exit /b 1
)

echo.
echo [2/2] Building...
cmake --build . --config Release --parallel

if %errorlevel% equ 0 (
    echo.
    echo ====================================================================
    echo Build Successful!
    echo ====================================================================
    echo.
    echo Copying DLLs...

    if exist Release\d3d12_hybrid_capture.dll copy Release\d3d12_hybrid_capture.dll ..\
    if exist Release\nvenc_full.dll copy Release\nvenc_full.dll ..\
    if exist Release\dxgi_capture.dll copy Release\dxgi_capture.dll ..\

    echo Done.
) else (
    echo.
    echo Build failed!
)

cd ..
pause
