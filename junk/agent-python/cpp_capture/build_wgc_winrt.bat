@echo off
setlocal

call "D:\Program Files\Microsoft Visual Studio\2022\Community\VC\Auxiliary\Build\vcvars64.bat"

REM 设置路径
set INCLUDE=D:\Program Files\Microsoft Visual Studio\2022\Community\VC\Tools\MSVC\14.44.35207\include;C:\Program Files (x86)\Windows Kits\10\Include\10.0.26100.0\ucrt;C:\Program Files (x86)\Windows Kits\10\Include\10.0.26100.0\shared;C:\Program Files (x86)\Windows Kits\10\Include\10.0.26100.0\um;C:\Program Files (x86)\Windows Kits\10\Include\10.0.26100.0\winrt
set LIB=D:\Program Files\Microsoft Visual Studio\2022\Community\VC\Tools\MSVC\14.44.35207\lib\x64;C:\Program Files (x86)\Windows Kits\10\Lib\10.0.26100.0\ucrt\x64;C:\Program Files (x86)\Windows Kits\10\Lib\10.0.26100.0\um\x64

echo ====================================================================
echo Building wgc_winrt.dll (C++/WinRT)
echo ====================================================================

REM C++/WinRT 需要 /await 和特定链接
cl.exe /LD /MD /O2 /EHsc /std:c++20 /await /ZW /DWGC_WINRT_EXPORTS ^
    wgc_winrt.cpp ^
    /link d3d11.lib dxgi.lib dwmapi.lib user32.lib windowsapp.lib ^
    /OUT:wgc_winrt.dll

if %errorlevel% equ 0 (
    echo.
    echo ====================================================================
    echo SUCCESS! wgc_winrt.dll created
    echo ====================================================================
    dir wgc_winrt.dll
) else (
    echo.
    echo ====================================================================
    echo FAILED!
    echo ====================================================================
)

endlocal
pause
