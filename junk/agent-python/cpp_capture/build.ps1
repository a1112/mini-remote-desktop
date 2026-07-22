# GPU Direct Build Script for PowerShell
# Usage: .\build.ps1

$ErrorActionPreference = "Stop"

Write-Host "====================================================================" -ForegroundColor Cyan
Write-Host "GPU Direct DLL Build Script" -ForegroundColor Cyan
Write-Host "====================================================================" -ForegroundColor Cyan
Write-Host ""

# Find Visual Studio installation
$vsWhere = "C:\Program Files (x86)\Microsoft Visual Studio\Installer\vswhere.exe"
if (Test-Path $vsWhere) {
    $vsPath = & $vsWhere -latest -property installationPath
    if ($vsPath) {
        Write-Host "[INFO] Found Visual Studio: $vsPath" -ForegroundColor Green

        # Import VS build environment
        $vcvars = "$vsPath\VC\Auxiliary\Build\vcvars64.bat"
        if (Test-Path $vcvars) {
            Write-Host "[INFO] Setting up VS build environment..." -ForegroundColor Yellow

            # Set environment variables
            $env:Path += ";$vsPath\VC\Tools\MSVC\*\bin\Hostx64\x64"
            $env:INCLUDE += ";$vsPath\VC\Tools\MSVC\*\include"
            $env:LIB += ";$vsPath\VC\Tools\MSVC\*\lib\x64"
        }
    }
}

# Set paths
$NVENC_SDK = "J:\ProjectTest\远程探查\mini-remote-desktop\tools\Video_Codec_Interface_13.0.37\Interface"
$CUDA_PATH = "C:\Program Files\NVIDIA GPU Computing Toolkit\CUDA\v12.6"

Write-Host "[INFO] NVENC SDK: $NVENC_SDK" -ForegroundColor Cyan
Write-Host "[INFO] CUDA:       $CUDA_PATH" -ForegroundColor Cyan
Write-Host ""

# Check cmake
$cmake = Get-Command cmake -ErrorAction SilentlyContinue
if (-not $cmake) {
    Write-Host "[ERROR] CMake not found. Please install CMake." -ForegroundColor Red
    exit 1
}
Write-Host "[INFO] CMake: $($cmake.Source)" -ForegroundColor Green
Write-Host ""

# Create build directory
$buildDir = "build"
if (-not (Test-Path $buildDir)) {
    New-Item -ItemType Directory -Path $buildDir | Out-Null
    Write-Host "[1/4] Created build directory" -ForegroundColor Green
} else {
    Write-Host "[1/4] Build directory exists" -ForegroundColor Yellow
}

# Configure CMake
Write-Host "[2/4] Configuring CMake..." -ForegroundColor Yellow
Push-Location $buildDir

$cmakeArgs = @(
    ".."
    "-G", "Ninja"
    "-DCMAKE_BUILD_TYPE=Release"
    "-DNVENC_SDK_PATH=$NVENC_SDK"
    "-DCUDA_PATH=$CUDA_PATH"
)

& cmake $cmakeArgs
if ($LASTEXITCODE -ne 0) {
    Write-Host "[WARNING] CMake with Ninja failed, trying Visual Studio generator..." -ForegroundColor Yellow
    $cmakeArgs = @(
        ".."
        "-G", "Visual Studio 17 2022"
        "-A", "x64"
        "-DCMAKE_BUILD_TYPE=Release"
        "-DNVENC_SDK_PATH=$NVENC_SDK"
        "-DCUDA_PATH=$CUDA_PATH"
    )
    & cmake $cmakeArgs
    if ($LASTEXITCODE -ne 0) {
        Write-Host "[ERROR] CMake configuration failed!" -ForegroundColor Red
        Pop-Location
        exit 1
    }
}

# Build
Write-Host "[3/4] Building DLLs..." -ForegroundColor Yellow
& cmake --build . --config Release --parallel
if ($LASTEXITCODE -ne 0) {
    Write-Host "[ERROR] Build failed!" -ForegroundColor Red
    Pop-Location
    exit 1
}

Pop-Location

# Copy DLLs
Write-Host "[4/4] Copying DLLs..." -ForegroundColor Yellow
$dlls = @(
    "build\lib\d3d12_hybrid_capture.dll",
    "build\lib\nvenc_full.dll",
    "build\lib\dxgi_capture.dll"
)

$copied = 0
foreach ($dll in $dlls) {
    if (Test-Path $dll) {
        Copy-Item $dll -Destination ".\" -Force
        Write-Host "  Copied: $(Split-Path $dll -Leaf)" -ForegroundColor Green
        $copied++
    } else {
        Write-Host "  Not found: $dll" -ForegroundColor Red
    }
}

Write-Host ""
Write-Host "====================================================================" -ForegroundColor Cyan
Write-Host "Build Complete!" -ForegroundColor Green
Write-Host "Copied $copied DLLs to current directory" -ForegroundColor Cyan
Write-Host "====================================================================" -ForegroundColor Cyan
