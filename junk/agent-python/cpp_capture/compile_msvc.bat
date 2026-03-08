@echo off
setlocal

REM 设置 VS 环境
set "VSPATH=D:\Program Files\Microsoft Visual Studio\2022\Community"
if exist "%VSPATH%\VC\Auxiliary\Build\vcvars64.bat" (
    call "%VSPATH%\VC\Auxiliary\Build\vcvars64.bat"
    goto :compile
)

REM 尝试其他可能的 VS 版本
set "VSPATH=C:\Program Files\Microsoft Visual Studio\2022\Community"
if exist "%VSPATH%\VC\Auxiliary\Build\vcvars64.bat" (
    call "%VSPATH%\VC\Auxiliary\Build\vcvars64.bat"
    goto :compile
)

echo ERROR: Cannot find Visual Studio 2022
echo Please update VSPATH in this script
pause
exit /b 1

:compile
cd /d "%~dp0"

echo.
echo ========================================================================
echo Compiling DXGI Capture DLL
echo ========================================================================
echo.

cl.exe /LD /MD /O2 /EHsc /std:c++17 /I"D:\Windows Kits\10\Include\10.0.22000.0\shared" /I"D:\Windows Kits\10\Include\10.0.22000.0\um" dxgi_capture.cpp /link d3d11.lib dxgi.lib /LIBPATH:"D:\Windows Kits\10\Lib\10.0.22000.0\um\x64" /OUT:dxgi_capture.dll

if %errorlevel% equ 0 (
    echo.
    echo ========================================================================
    echo SUCCESS!
    echo ========================================================================
    if exist dxgi_capture.dll (
        echo DLL created: dxgi_capture.dll
        copy /Y dxgi_capture.dll ..\ >nul
        echo Copied to project root.
    )
    dir dxgi_capture.*
) else (
    echo.
    echo ========================================================================
    echo FAILED!
    echo ========================================================================
)

endlocal
pause
