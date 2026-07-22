@echo off
setlocal

call "D:\Program Files\Microsoft Visual Studio\2022\Community\VC\Auxiliary\Build\vcvars64.bat"

set INCLUDE=D:\Program Files\Microsoft Visual Studio\2022\Community\VC\Tools\MSVC\14.44.35207\include;C:\Program Files (x86)\Windows Kits\10\Include\10.0.26100.0\ucrt;C:\Program Files (x86)\Windows Kits\10\Include\10.0.26100.0\shared;C:\Program Files (x86)\Windows Kits\10\Include\10.0.26100.0\um
set LIB=D:\Program Files\Microsoft Visual Studio\2022\Community\VC\Tools\MSVC\14.44.35207\lib\x64;C:\Program Files (x86)\Windows Kits\10\Lib\10.0.26100.0\ucrt\x64;C:\Program Files (x86)\Windows Kits\10\Lib\10.0.26100.0\um\x64

echo ====================================================================
echo Compiling with DXGI 1.5 support...
echo ====================================================================

cl.exe /LD /MD /O2 /EHsc /DD3D12_HYBRID_CAPTURE_EXPORTS ^
    d3d12_hybrid_capture.cpp ^
    /link d3d11.lib d3d12.lib dxgi.lib /OUT:d3d12_hybrid_capture.dll

if %errorlevel% equ 0 (
    echo.
    echo ====================================================================
    Compilation successful!
    echo ====================================================================
    echo.
    dir d3d12_hybrid_capture.dll
) else (
    echo.
    echo ====================================================================
    Compilation failed!
    echo ====================================================================
)

endlocal
pause
