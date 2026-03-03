@echo off
setlocal

REM Setup VS environment
call "D:\Program Files\Microsoft Visual Studio\2022\Community\VC\Auxiliary\Build\vcvars64.bat"

REM Set include paths
set INCLUDE=D:\Program Files\Microsoft Visual Studio\2022\Community\VC\Tools\MSVC\14.44.35207\include;C:\Program Files (x86)\Windows Kits\10\Include\10.0.26100.0\ucrt;C:\Program Files (x86)\Windows Kits\10\Include\10.0.26100.0\shared;C:\Program Files (x86)\Windows Kits\10\Include\10.0.26100.0\um

REM Set lib paths
set LIB=D:\Program Files\Microsoft Visual Studio\2022\Community\VC\Tools\MSVC\14.44.35207\lib\x64;C:\Program Files (x86)\Windows Kits\10\Lib\10.0.26100.0\ucrt\x64;C:\Program Files (x86)\Windows Kits\10\Lib\10.0.26100.0\um\x64

echo ====================================================================
echo Compiling d3d12_hybrid_capture.dll
echo ====================================================================

cl.exe /LD /MD /O2 /EHsc /DD3D12_HYBRID_CAPTURE_EXPORTS ^
    d3d12_hybrid_capture.cpp ^
    /link d3d11.lib d3d12.lib dxgi.lib /OUT:d3d12_hybrid_capture.dll

if %errorlevel% equ 0 (
    echo.
    echo ====================================================================
    echo d3d12_hybrid_capture.dll compiled successfully!
    echo ====================================================================
) else (
    echo.
    echo ====================================================================
    echo Compilation failed!
    echo ====================================================================
)

endlocal
pause
