@echo off
call "D:\Program Files\Microsoft Visual Studio\2022\Community\VC\Auxiliary\Build\vcvars64.bat"
"C:\Program Files\NVIDIA GPU Computing Toolkit\CUDA\v13.0\bin\nvcc.exe" -ptx -m=64 -O3 -arch=sm_75 "J:\ProjectTest\远程探查\mini-remote-desktop\agent-python\cpp_capture\bgra_to_nv12.cu" -o "J:\ProjectTest\远程探查\mini-remote-desktop\agent-python\cpp_capture\bgra_to_nv12.ptx"
type "J:\ProjectTest\远程探查\mini-remote-desktop\agent-python\cpp_capture\bgra_to_nv12.ptx" | findstr /C:"//" /C:"."
