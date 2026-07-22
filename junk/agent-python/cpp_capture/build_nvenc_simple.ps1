# Simple build script for nvenc_full.dll
Set-Location "J:\ProjectTest\远程探查\mini-remote-desktop\agent-python\cpp_capture"

$batFile = Join-Path $PWD "do_build_nvenc.bat"
$output = & cmd.exe /c "`"$batFile`" 2>&1" | Out-String
Write-Host $output

if (Test-Path ".\nvenc_full.dll") {
    Write-Host "=== DLL CREATED SUCCESSFULLY ==="
    Get-Item ".\nvenc_full.dll" | Format-List
    exit 0
} else {
    Write-Host "=== DLL NOT FOUND ==="
    exit 1
}
