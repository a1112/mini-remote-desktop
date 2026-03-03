@echo off
REM 构建 DXGI Capture DLL

echo ========================================================================
echo DXGI Desktop Duplication DLL Build Script
echo ========================================================================
echo.

REM 检查 Visual Studio
where cl >nul 2>&1
if %errorlevel% neq 0 (
    echo 错误: 未找到 Visual Studio C++ 编译器
    echo.
    echo 请安装 Visual Studio 2022 并包含 "使用 C++ 的桌面开发" 工作负载
    echo.
    echo 或使用 Developer Command Prompt:
    echo   开始菜单 ^> Visual Studio 2022 ^> Developer Command Prompt
    echo.
    pause
    exit /b 1
)

REM 设置构建目录
set BUILD_DIR=build
if not exist %BUILD_DIR% mkdir %BUILD_DIR%

echo 配置 CMake...
cd /d %~dp0

REM 方法 1: 使用 CMake (推荐)
where cmake >nul 2>&1
if %errorlevel% equ 0 (
    echo 使用 CMake 构建...
    cmake -B %BUILD_DIR% -A x64
    if %errorlevel% neq 0 (
        echo CMake 配置失败
        pause
        exit /b 1
    )

    echo.
    echo 编译...
    cmake --build %BUILD_DIR% --config Release
    if %errorlevel% equ 0 (
        echo.
        echo ========================================================================
        echo 构建成功!
        echo ========================================================================
        echo DLL 位置: %BUILD_DIR%\bin\Release\dxgi_capture.dll
        echo.
        echo 复制到项目根目录...
        copy /Y %BUILD_DIR%\bin\Release\dxgi_capture.dll ..\dxgi_capture.dll
        echo 完成!
        echo.
    ) else (
        echo 编译失败
        pause
        exit /b 1
    )
) else (
    echo CMake 未找到，使用直接编译...
    echo.

    REM 方法 2: 直接使用 cl 编译
    cl.exe /LD /MD /O2 /EHsc ^
        /I"C:\Program Files (x86)\Windows Kits\10\Include\10.0.22000.0\shared" ^
        /I"C:\Program Files (x86)\Windows Kits\10\Include\10.0.22000.0\um" ^
        /I"C:\Program Files (x86)\Windows Kits\10\Include\10.0.22000.0\winrt" ^
        dxgi_capture.cpp ^
        /link d3d11.lib dxgi.lib /OUT:dxgi_capture.dll

    if %errorlevel% equ 0 (
        echo.
        echo ========================================================================
        echo 构建成功!
        echo ========================================================================
        echo.
        echo 复制到项目根目录...
        copy /Y dxgi_capture.dll ..\
        echo 完成!
        echo.
    )
)

pause
