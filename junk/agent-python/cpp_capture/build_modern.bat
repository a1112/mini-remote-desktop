@echo off
setlocal

REM Setup VS environment
call "D:\Program Files\Microsoft Visual Studio\2022\Community\VC\Auxiliary\Build\vcvars64.bat"

REM Set include paths
set INCLUDE=D:\Program Files\Microsoft Visual Studio\2022\Community\VC\Tools\MSVC\14.44.35207\include;C:\Program Files (x86)\Windows Kits\10\Include\10.0.26100.0\ucrt;C:\Program Files (x86)\Windows Kits\10\Include\10.0.26100.0\shared;C:\Program Files (x86)\Windows Kits\10\Include\10.0.26100.0\um

REM Set lib paths
set LIB=D:\Program Files\Microsoft Visual Studio\2022\Community\VC\Tools\MSVC\14.44.35207\lib\x64;C:\Program Files (x86)\Windows Kits\10\Lib\10.0.26100.0\ucrt\x64;C:\Program Files (x86)\Windows Kits\10\Lib\10.0.26100.0\um\x64

echo ====================================================================
echo Compiling modern_dxgi_capture.dll
echo ====================================================================

cl.exe /LD /MD /O2 /EHsc ^
    modern_dxgi_capture.cpp ^
    /link d3d11.lib dxgi.lib dxguid.lib /OUT:modern_dxgi_capture.dll

if %errorlevel% equ 0 (
    echo.
    echo ====================================================================
    modern_dxgi_capture.dll compiled successfully!
    echo ====================================================================

    REM Test it
    echo.
    echo Testing modern DXGI...
    python -c "import ctypes; dll = ctypes.CDLL('modern_dxgi_capture.dll'); print('Result:', dll.test_modern_dxgi())"
) else (
    echo.
    echo ====================================================================
    echo Compilation failed!
    echo ====================================================================
)

endlocal
pause
