@echo off
setlocal

REM Setup VS environment
call "D:\Program Files\Microsoft Visual Studio\2022\Community\VC\Auxiliary\Build\vcvars64.bat"

REM Set include paths
set INCLUDE=D:\Program Files\Microsoft Visual Studio\2022\Community\VC\Tools\MSVC\14.44.35207\include;C:\Program Files (x86)\Windows Kits\10\Include\10.0.26100.0\ucrt;C:\Program Files (x86)\Windows Kits\10\Include\10.0.26100.0\shared;C:\Program Files (x86)\Windows Kits\10\Include\10.0.26100.0\um;J:\ProjectTest\远程探查\mini-remote-desktop\tools\Video_Codec_Interface_13.0.37\Interface;C:\Program Files\NVIDIA GPU Computing Toolkit\CUDA\v12.6\include

REM Set lib paths
set LIB=D:\Program Files\Microsoft Visual Studio\2022\Community\VC\Tools\MSVC\14.44.35207\lib\x64;C:\Program Files (x86)\Windows Kits\10\Lib\10.0.26100.0\ucrt\x64;C:\Program Files (x86)\Windows Kits\10\Lib\10.0.26100.0\um\x64;C:\Program Files\NVIDIA GPU Computing Toolkit\CUDA\v12.6\lib\x64

echo ====================================================================
echo Compiling nvenc_full.dll
echo ====================================================================

cl.exe /LD /MD /O2 /EHsc /DNVENC_ENCODER_EXPORTS ^
    nvenc_full.cpp ^
    /link d3d11.lib cuda.lib nvencodeapi.lib /OUT:nvenc_full.dll

if %errorlevel% equ 0 (
    echo.
    echo ====================================================================
    echo nvenc_full.dll compiled successfully!
    echo ====================================================================
) else (
    echo.
    echo ====================================================================
    echo Compilation failed!
    echo ====================================================================
)

endlocal
pause
