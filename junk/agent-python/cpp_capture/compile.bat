@echo off
setlocal EnableDelayedExpansion

REM 设置编译环境路径
set VS_DEV=D:\Program Files\Microsoft Visual Studio\2022\Community
set MSVC=D:\Program Files\Microsoft Visual Studio\2022\Community\VC\Tools\MSVC\14.44.35207
set WIN_SDK=C:\Program Files (x86)\Windows Kits\10

REM 设置环境变量
set INCLUDE=%MSVC%\include;%WIN_SDK%\Include\10.0.26100.0\ucrt;%WIN_SDK%\Include\10.0.26100.0\shared;%WIN_SDK%\Include\10.0.26100.0\um
set LIB=%MSVC%\lib\x64;%WIN_SDK%\Lib\10.0.26100.0\ucrt\x64;%WIN_SDK%\Lib\10.0.26100.0\um\x64
set PATH=%MSVC%\bin\Hostx64\x64;%PATH%

echo ====================================================================
echo Compiling with DXGI 1.5 Support
echo ====================================================================

REM 调用 vcvars64.bat 设置完整环境
call "%VS_DEV%\VC\Auxiliary\Build\vcvars64.bat" >nul 2>&1

REM 编译
cl.exe /LD /MD /O2 /EHsc /DD3D12_HYBRID_CAPTURE_EXPORTS ^
    d3d12_hybrid_capture.cpp ^
    /link d3d11.lib d3d12.lib dxgi.lib /OUT:d3d12_hybrid_capture.dll

if %errorlevel% equ 0 (
    echo.
    echo ====================================================================
    echo SUCCESS! DLL created.
    echo ====================================================================
    dir d3d12_hybrid_capture.dll
) else (
    echo.
    echo ====================================================================
    echo FAILED!
    echo ====================================================================
)

endlocal
