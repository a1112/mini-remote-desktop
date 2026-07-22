# Build nvenc_full.dll with ABGR format support
Set-Location "J:\ProjectTest\远程探查\mini-remote-desktop\agent-python\cpp_capture"

$batContent = @"
@echo off
call "D:\Program Files\Microsoft Visual Studio\2022\Community\VC\Auxiliary\Build\vcvars64.bat"
cd /d J:\ProjectTest\远程探查\mini-remote-desktop\agent-python\cpp_capture
echo Compiling nvenc_full.cpp...
cl /LD /EHsc /std:c++20 /I".\nvenc_headers" /I"C:\Program Files\NVIDIA GPU Computing Toolkit\CUDA\v13.0\include" nvenc_full.cpp /link d3d11.lib dxgi.lib cuda.lib cudart.lib /LIBPATH:"C:\Program Files\NVIDIA GPU Computing Toolkit\CUDA\v13.0\lib\x64" /OUT:nvenc_full.dll
if errorlevel 1 exit /b 1
dir nvenc_full.dll
"@

$batFile = Join-Path $PWD "build_nvenc_abgr.bat"
$batContent | Out-File -FilePath $batFile -Encoding ASCII

$result = cmd.exe /c "`"$batFile`""
Write-Host $result

if (Test-Path ".\nvenc_full.dll") {
    Write-Host "=== DLL CREATED SUCCESSFULLY ==="
    Get-Item ".\nvenc_full.dll" | Format-List
    exit 0
} else {
    Write-Host "=== DLL NOT FOUND ==="
    exit 1
}
