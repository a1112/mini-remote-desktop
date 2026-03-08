@echo off
call "D:\Program Files\Microsoft Visual Studio\2022\Community\VC\Auxiliary\Build\vcvars64.bat"
chcp 65001 > nul
cd /d "J:\ProjectTest\????\mini-remote-desktop\agent-python\cpp_capture"
echo Compiling wgc_simple.cpp...
cl /LD /O2 /EHsc /std:c++20 /DWGC_EXPORTS wgc_simple.cpp /link d3d11.lib dxgi.lib dwmapi.lib user32.lib /OUT:wgc_capture.dll
if errorlevel 1 (
    echo FAILED: Compilation error
    exit /b 1
)
echo SUCCESS: wgc_capture.dll created
dir wgc_capture.dll
