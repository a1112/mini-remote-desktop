param(
    [string]$ListenAddress = '127.0.0.1:9534',
    [string]$Executable = '',
    [switch]$Check
)
$ErrorActionPreference = 'Stop'
if (!$Executable) {
    $repoRoot = (Resolve-Path (Join-Path $PSScriptRoot '../..')).Path
    $buildRoot = if ($env:CARGO_TARGET_DIR) { $env:CARGO_TARGET_DIR } else { Join-Path $repoRoot 'target' }
    $Executable = Join-Path $buildRoot 'debug/mrd-mobile-gateway.exe'
}
if (!(Test-Path -LiteralPath $Executable -PathType Leaf)) {
    throw "Gateway missing: $Executable. Run cargo build -p mrd-mobile-gateway-app first."
}
# Some NVIDIA DCH installations only ship the NVENC DLL in DriverStore.
# Add that installed driver directory to this process, never the system PATH.
$driverRoot = Join-Path $env:SystemRoot 'System32/DriverStore/FileRepository'
$encoderDirectory = Get-ChildItem -LiteralPath $driverRoot -Directory -Filter 'nv_disp*.inf_amd64*' |
    Where-Object { Test-Path -LiteralPath (Join-Path $_.FullName 'nvEncodeAPI64.dll') } |
    Sort-Object LastWriteTime -Descending | Select-Object -First 1
$previousPath = $env:PATH
$previousBind = $env:MRD_MOBILE_GATEWAY_BIND
try {
    if ($encoderDirectory) { $env:PATH = "$($encoderDirectory.FullName);$previousPath" }
    $env:MRD_MOBILE_GATEWAY_BIND = $ListenAddress
    if ($Check) {
        [pscustomobject]@{ Executable = $Executable; Bind = $ListenAddress; EncoderDirectory = $encoderDirectory.FullName }
    } else {
        & $Executable
        if ($LASTEXITCODE -ne 0) { throw "Gateway exited with code $LASTEXITCODE" }
    }
} finally {
    $env:PATH = $previousPath
    $env:MRD_MOBILE_GATEWAY_BIND = $previousBind
}
