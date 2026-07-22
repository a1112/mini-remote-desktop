@echo off
call "D:\Program Files\Microsoft Visual Studio\2022\Community\VC\Auxiliary\Build\vcvars64.bat"
cd /d J:\ProjectTest\远程探查\mini-remote-desktop\agent-python\cpp_capture
cl /LD /O2 /EHsc /std:c++20 /DWGC_EXPORTS wgc_simple.cpp /link d3d11.lib dxgi.lib dwmapi.lib /OUT:wgc_capture.dll
dir wgc_capture.dll
