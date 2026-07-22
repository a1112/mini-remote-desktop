@echo off
set LOGFILE=build_log.txt
echo Build started at %date% %time% > %LOGFILE%

call "C:\Program Files\Microsoft Visual Studio\2022\Community\VC\Auxiliary\Build\vcvars64.bat" >> %LOGFILE% 2>&1

cd /d "J:\ProjectTest\远程探查\mini-remote-desktop\agent-python\cpp_capture"

echo Running CL.EXE... >> %LOGFILE%
cl /LD /EHsc /std:c++20 /I"J:\ProjectTest\远程探查\mini-remote-desktop\tools\Video_Codec_Interface_13.0.37\Interface" /I"C:\Program Files\NVIDIA GPU Computing Toolkit\CUDA\v12.6\include" nvenc_full.cpp /link d3d11.lib cuda.lib cudart.lib nvEncodeAPI64.lib /LIBPATH:"C:\Program Files\NVIDIA GPU Computing Toolkit\CUDA\v12.6\lib\x64" /OUT:nvenc_full.dll >> %LOGFILE% 2>&1

echo Exit code: %ERRORLEVEL% >> %LOGFILE%

dir nvenc_full.dll >> %LOGFILE% 2>&1

type %LOGFILE%
