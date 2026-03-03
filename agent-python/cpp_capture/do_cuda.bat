call "D:\Program Files\Microsoft Visual Studio\2022\Community\VC\Auxiliary\Build\vcvars64.bat"
"C:\Program Files\NVIDIA GPU Computing Toolkit\CUDA\v13.0\bin\nvcc.exe" -ptx -m=64 -O3 -arch=sm_75 bgra_to_nv12.cu -o bgra_to_nv12.ptx
