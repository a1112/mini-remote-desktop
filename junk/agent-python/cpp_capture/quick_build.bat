@echo off
call "D:\Program Files\Microsoft Visual Studio\2022\Community\VC\Auxiliary\Build\vcvars64.bat"
cd /d "J:\ProjectTest\远程探查\mini-remote-desktop\agent-python\cpp_capture"
echo Compiling nvenc_full.dll with UNDEFINED format...
cl /LD /EHsc /std:c++20 /O2 /I"J:\ProjectTest\远程探查\mini-remote-desktop\tools\Video_Codec_Interface_13.0.37\Interface" /I"C:\Program Files\NVIDIA GPU Computing Toolkit\CUDA\v13.0\include" nvenc_full.cpp /link d3d11.lib cuda.lib cudart.lib /LIBPATH:"C:\Program Files\NVIDIA GPU Computing Toolkit\CUDA\v13.0\lib\x64" /OUT:nvenc_full.dll
echo Done!
