@echo off
echo Setting up Visual Studio environment...
call "D:\Program Files\Microsoft Visual Studio\2022\Community\VC\Auxiliary\Build\vcvars64.bat"

echo.
echo Compiling dxgi_capture.dll...
echo.

cd /d "%~dp0"

cl.exe /LD /MD /O2 /EHsc /std:c++17 dxgi_capture.cpp /link d3d11.lib dxgi.lib /OUT:dxgi_capture.dll

if %errorlevel% equ 0 (
    echo.
    echo ========================================================================
    echo 编译成功!
    echo ========================================================================
    copy /Y dxgi_capture.dll ..\
    echo DLL 已复制到项目根目录
    echo.
    dir dxgi_capture.dll
) else (
    echo.
    echo ========================================================================
    echo 编译失败!
    echo ========================================================================
)

pause
