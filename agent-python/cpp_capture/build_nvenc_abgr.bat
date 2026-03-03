@echo off
call "D:\Program Files\Microsoft Visual Studio\2022\Community\VC\Auxiliary\Build\vcvars64.bat"
cd /d J:\ProjectTest\????\mini-remote-desktop\agent-python\cpp_capture
echo Compiling nvenc_full.cpp...
cl /LD /EHsc /std:c++20 /I".\nvenc_headers" /I"C:\Program Files\NVIDIA GPU Computing Toolkit\CUDA\v13.0\include" nvenc_full.cpp /link d3d11.lib dxgi.lib cuda.lib cudart.lib /LIBPATH:"C:\Program Files\NVIDIA GPU Computing Toolkit\CUDA\v13.0\lib\x64" /OUT:nvenc_full.dll
if errorlevel 1 exit /b 1
dir nvenc_full.dll
