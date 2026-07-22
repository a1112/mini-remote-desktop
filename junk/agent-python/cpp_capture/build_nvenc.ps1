# Build script for nvenc_full.dll
$ErrorActionPreference = "Continue"

# Find Visual Studio installation
$vsPath = & "${env:ProgramFiles(x86)}\Microsoft Visual Studio\Installer\vswhere.exe" `
    -latest -property installationPath `
    -products * `
    -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64

if (-not $vsPath) {
    Write-Error "Visual Studio not found"
    exit 1
}

Write-Host "VS Path: $vsPath"

# Import VC module
Import-Module (Join-Path $vsPath "Common7\IDE\Microsoft.VisualStudio.DevShell.dll")
Enter-VsDevShell -VsInstallPath $vsPath -SkipAutomaticLocation

# Change to cpp_capture directory
Set-Location "J:\ProjectTest\远程探查\mini-remote-desktop\agent-python\cpp_capture"

# Compile
cl /LD /EHsc /std:c++20 `
    /I"J:\ProjectTest\远程探查\mini-remote-desktop\tools\Video_Codec_Interface_13.0.37\Interface" `
    /I"C:\Program Files\NVIDIA GPU Computing Toolkit\CUDA\v12.6\include" `
    nvenc_full.cpp `
    /link d3d11.lib cuda.lib cudart.lib nvEncodeAPI64.lib `
    /LIBPATH:"C:\Program Files\NVIDIA GPU Computing Toolkit\CUDA\v12.6\lib\x64" `
    /OUT:nvenc_full.dll

if ($LASTEXITCODE -eq 0) {
    Write-Host "SUCCESS: nvenc_full.dll created"
    Get-Item nvenc_full.dll | Format-List
} else {
    Write-Error "FAILED: Compilation error (exit code: $LASTEXITCODE)"
    exit $LASTEXITCODE
}
