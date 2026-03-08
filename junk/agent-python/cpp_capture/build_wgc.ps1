# Build WGC Capture DLL

Write-Host "Building wgc_capture.dll..."

# Set VS environment
$vsDevCmd = "D:\Program Files\Microsoft Visual Studio\2022\Community\VC\Auxiliary\Build\vcvars64.bat"
$env:VSCMD_START_DIR = "J:\ProjectTest\远程探查\mini-remote-desktop\agent-python\cpp_capture"

# Create batch content
$batchContent = @"
@echo off
call "$vsDevCmd"
chcp 65001 > nul
cd /d "J:\ProjectTest\远程探查\mini-remote-desktop\agent-python\cpp_capture"
echo Compiling wgc_simple.cpp...
cl /LD /O2 /EHsc /std:c++20 /DWGC_EXPORTS wgc_simple.cpp /link d3d11.lib dxgi.lib dwmapi.lib user32.lib /OUT:wgc_capture.dll
if errorlevel 1 (
    echo FAILED: Compilation error
    exit /b 1
)
echo SUCCESS: wgc_capture.dll created
dir wgc_capture.dll
"@

$batchFile = "J:\ProjectTest\远程探查\mini-remote-desktop\agent-python\cpp_capture\build_wgc_inline.bat"
$batchContent | Out-File -FilePath $batchFile -Encoding ASCII

# Run batch file
$result = cmd.exe /c "`"$batchFile`""
Write-Host $result

# Check DLL
if (Test-Path "J:\ProjectTest\远程探查\mini-remote-desktop\agent-python\cpp_capture\wgc_capture.dll") {
    Write-Host "wgc_capture.dll created successfully"
    Get-Item "J:\ProjectTest\远程探查\mini-remote-desktop\agent-python\cpp_capture\wgc_capture.dll" | Format-List
}
