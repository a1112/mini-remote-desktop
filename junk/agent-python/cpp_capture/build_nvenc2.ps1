# Build script for nvenc_full.dll
$ErrorActionPreference = "Continue"

# Change to cpp_capture directory first
Set-Location "J:\ProjectTest\远程探查\mini-remote-desktop\agent-python\cpp_capture"

# Set up VS environment using cmd
$vsDevCmd = "D:\Program Files\Microsoft Visual Studio\2022\Community\VC\Auxiliary\Build\vcvars64.bat"

# Use local directory for NVENC headers
$nvencInclude = ".\nvenc_headers"

# Create a temporary batch file to run cl.exe
# Using short paths and local directories to avoid character encoding issues
$batchContent = @"
@echo off
call "$vsDevCmd"
cd /d J:\ProjectTest\远程探查\mini-remote-desktop\agent-python\cpp_capture
cl /LD /EHsc /std:c++20 /I"$nvencInclude" /I"C:\Program Files\NVIDIA GPU Computing Toolkit\CUDA\v13.0\include" nvenc_full.cpp /link d3d11.lib cuda.lib cudart.lib nvEncodeAPI64.lib /LIBPATH:"C:\Program Files\NVIDIA GPU Computing Toolkit\CUDA\v13.0\lib\x64" /OUT:nvenc_full.dll
exit /b %ERRORLEVEL%
"@

$batchFile = "temp_build.bat"
$batchContent | Out-File -FilePath $batchFile -Encoding ASCII

# Run the batch file
$process = Start-Process cmd.exe -ArgumentList "/c", "`"$batchFile`"" -Wait -NoNewWindow -PassThru

# Check result
if ($process.ExitCode -eq 0) {
    Write-Host "SUCCESS: nvenc_full.dll created"
    Get-Item nvenc_full.dll -ErrorAction SilentlyContinue | Format-List
} else {
    Write-Error "FAILED: Compilation error (exit code: $($process.ExitCode))"
}

# Clean up
Remove-Item $batchFile -ErrorAction SilentlyContinue
exit $process.ExitCode
