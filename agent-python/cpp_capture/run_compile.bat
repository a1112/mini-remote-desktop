@echo off
setlocal enabledelayedexpansion

REM Visual Studio 2022 x64 Native Tools Command Prompt
call "D:\Program Files\Microsoft Visual Studio\2022\Community\VC\Auxiliary\Build\vcvars64.bat"

REM Change to script directory
cd /d "%~dp0"

REM Compile
echo Compiling...
cl.exe /LD /MD /O2 /EHsc /std:c++17 dxgi_capture.cpp /link d3d11.lib dxgi.lib /OUT:dxgi_capture.dll

REM Check result
if exist dxgi_capture.dll (
    echo SUCCESS!
    dir dxgi_capture.dll
    copy /Y dxgi_capture.dll ..\
) else (
    echo FAILED!
)

endlocal
