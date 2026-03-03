@echo off
setlocal
call "D:\Program Files\Microsoft Visual Studio\2022\Community\VC\Auxiliary\Build\vcvars64.bat" >nul 2>&1
call "C:\Program Files\NVIDIA GPU Computing Toolkit\CUDA\v13.0\bin\nvcc.exe" -ptx -m=64 -O3 -arch=sm_75 bgra_to_nv12.cu -o bgra_to_nv12.ptx
if exist bgra_to_nv12.ptx (
    echo SUCCESS: PTX file created
    dir bgra_to_nv12.ptx
) else (
    echo FAILED: PTX file not created
)
endlocal
