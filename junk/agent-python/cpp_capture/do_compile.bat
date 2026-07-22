@echo off
call "D:\Program Files\Microsoft Visual Studio\2022\Community\VC\Auxiliary\Build\vcvars64.bat"
cd /d "J:\ProjectTest\远程探查\mini-remote-desktop\agent-python\cpp_capture"
cl.exe /LD /MD /O2 /EHsc /std:c++17 dxgi_capture.cpp /link d3d11.lib dxgi.lib /OUT:dxgi_capture.dll
copy /Y dxgi_capture.dll ..\dxgi_capture.dll
echo Done!
