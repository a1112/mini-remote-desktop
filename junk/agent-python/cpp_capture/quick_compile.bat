@echo off
call "C:\Program Files\Microsoft Visual Studio\2022\Community\VC\Auxiliary\Build\vcvars64.bat" > nul
echo Starting compilation...
cl /LD /EHsc /std:c++20 /O2 /I"J:\ProjectTest\远程探查\mini-remote-desktop\tools\Video_Codec_Interface_13.0.37\Interface" /I"C:\Program Files\NVIDIA GPU Computing Toolkit\CUDA\v12.6\include" nvenc_full.cpp /link d3d11.lib cuda.lib cudart.lib nvEncodeAPI64.lib /LIBPATH:"C:\Program Files\NVIDIA GPU Computing Toolkit\CUDA\v12.6\lib\x64" /OUT:nvenc_full.dll
echo Compilation exit code: %ERRORLEVEL%
if exist nvenc_full.dll echo DLL created successfully
