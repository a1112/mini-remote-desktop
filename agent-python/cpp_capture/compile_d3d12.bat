@echo off
REM ============================================================================
REM D3D12 混合捕获编译脚本
REM ============================================================================

setlocal enabledelayedexpansion

echo.
echo ============================================================
echo   D3D12 Hybrid Capture - Compile Script
echo ============================================================
echo.

REM 设置 VS 路径
set VS_PATH=D:\Program Files\Microsoft Visual Studio\2022\Community

REM 检查 VS 是否存在
if not exist "%VS_PATH%\VC\Auxiliary\Build\vcvars64.bat" (
    echo [ERROR] Visual Studio 2022 not found at %VS_PATH%
    echo Please modify VS_PATH in this script
    pause
    exit /b 1
)

REM 设置 Windows SDK 路径 (自动检测)
for /f "tokens=*" %%i in ('dir /b "C:\Program Files (x86)\Windows Kits\10\bin\*"') do (
    set SDK_VERSION=%%i
)

echo [INFO] Using Windows SDK: %SDK_VERSION%
echo [INFO] Using Visual Studio: %VS_PATH%
echo.

REM 初始化 VS 环境
call "%VS_PATH%\VC\Auxiliary\Build\vcvars64.bat"

REM 编译选项
set COMMON_FLAGS=/LD /MD /O2 /EHsc /std:c++17
set DX11_LIBS=d3d11.lib dxgi.lib
set DX12_LIBS=d3d12.lib D3D11On12.lib
set OUT_LIB=/LIBPATH:"C:\Program Files (x86)\Windows Kits\10\lib\%SDK_VERSION%\um\x64"
set INC_DIRS=/I"C:\Program Files (x86)\Windows Kits\10\include\%SDK_VERSION%\um"
set INC_DIRS=%INC_DIRS% /I"C:\Program Files (x86)\Windows Kits\10\include\%SDK_VERSION%\shared"

echo.
echo ============================================================
echo   Compiling: d3d12_hybrid_capture.dll
echo ============================================================
echo.

cl.exe %COMMON_FLAGS% %INC_DIRS% ^
    d3d12_hybrid_capture.cpp ^
    /link %DX11_LIBS% %DX12_LIBS% %OUT_LIB% ^
    /OUT:d3d12_hybrid_capture.dll

if %ERRORLEVEL% EQU 0 (
    echo.
    echo ============================================================
    echo   [SUCCESS] d3d12_hybrid_capture.dll compiled
    echo ============================================================
    dir d3d12_hybrid_capture.dll
) else (
    echo.
    echo ============================================================
    echo   [FAILED] Compilation errors
    echo ============================================================
    pause
    exit /b 1
)

echo.
pause
