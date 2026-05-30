param(
    [string]$Version = "v1.14.0",
    [ValidateSet("Release", "Debug", "RelWithDebInfo", "MinSizeRel")]
    [string]$Configuration = "Release",
    [string]$Generator = "",
    [string]$Architecture = "x64",
    [int]$Jobs = 0,
    [switch]$CheckOnly,
    [switch]$Force
)

$ErrorActionPreference = "Stop"

function Resolve-RepoPath {
    param([string]$Path)

    return [System.IO.Path]::GetFullPath((Join-Path $PSScriptRoot $Path))
}

function Assert-ChildPath {
    param(
        [string]$Root,
        [string]$Child
    )

    $rootPath = [System.IO.Path]::GetFullPath($Root).TrimEnd('\', '/')
    $childPath = [System.IO.Path]::GetFullPath($Child).TrimEnd('\', '/')
    $comparison = [System.StringComparison]::OrdinalIgnoreCase
    if (-not $childPath.StartsWith($rootPath + [System.IO.Path]::DirectorySeparatorChar, $comparison) -and
        -not [string]::Equals($rootPath, $childPath, $comparison)) {
        throw "Refusing to operate outside VVenC root: $childPath"
    }
}

function Require-Command {
    param([string]$Name)

    $command = Get-Command $Name -ErrorAction SilentlyContinue
    if (-not $command) {
        throw "Required command not found in PATH: $Name"
    }
    return $command.Source
}

function Find-PkgConfigDir {
    param([string]$InstallDir)

    $candidates = @(
        (Join-Path $InstallDir "lib\pkgconfig"),
        (Join-Path $InstallDir "lib64\pkgconfig")
    )
    foreach ($candidate in $candidates) {
        if (Test-Path (Join-Path $candidate "libvvenc.pc")) {
            return [System.IO.Path]::GetFullPath($candidate)
        }
    }
    throw "libvvenc.pc was not found under $InstallDir"
}

function Add-EnvPath {
    param(
        [string]$Name,
        [string]$Value
    )

    if (-not (Test-Path $Value)) {
        return
    }
    $separator = [System.IO.Path]::PathSeparator
    $current = [Environment]::GetEnvironmentVariable($Name, "Process")
    $parts = @()
    if ($current) {
        $parts = $current -split [regex]::Escape([string]$separator)
    }
    if ($parts -notcontains $Value) {
        [Environment]::SetEnvironmentVariable($Name, ($Value + $separator + $current), "Process")
    }
}

function Write-EnvFile {
    param(
        [string]$EnvFile,
        [string]$PkgConfigDir,
        [string]$InstallDir
    )

    $binDir = Join-Path $InstallDir "bin"
    $libDir = Join-Path $InstallDir "lib"
    $content = @"
`$vvencPkgConfigDir = "$PkgConfigDir"
`$vvencBinDir = "$binDir"
`$vvencLibDir = "$libDir"
`$pathSeparator = [System.IO.Path]::PathSeparator
if (`$env:PKG_CONFIG_PATH) {
    `$env:PKG_CONFIG_PATH = "`$vvencPkgConfigDir`$pathSeparator`$env:PKG_CONFIG_PATH"
} else {
    `$env:PKG_CONFIG_PATH = `$vvencPkgConfigDir
}
`$pathEntries = @(`$vvencBinDir, `$vvencLibDir) | Where-Object { Test-Path `$_ }
`$pathEntries = @(`$pathEntries)
[array]::Reverse(`$pathEntries)
foreach (`$entry in `$pathEntries) {
    if (`$env:PATH) {
        `$env:PATH = "`$entry`$pathSeparator`$env:PATH"
    } else {
        `$env:PATH = `$entry
    }
}
"@
    Set-Content -LiteralPath $EnvFile -Value $content -Encoding ASCII
}

function Select-CMakeGenerator {
    param([string]$Requested)

    if ($Requested) {
        return $Requested
    }
    if ((Get-Command cl -ErrorAction SilentlyContinue) -and (Get-Command ninja -ErrorAction SilentlyContinue)) {
        return "Ninja"
    }
    return "Visual Studio 17 2022"
}

$rootDir = [System.IO.Path]::GetFullPath($PSScriptRoot)
$sourceDir = Resolve-RepoPath "src"
$buildDir = Resolve-RepoPath "build"
$installDir = Resolve-RepoPath "install"
$envFile = Resolve-RepoPath "env.local.ps1"

Assert-ChildPath -Root $rootDir -Child $sourceDir
Assert-ChildPath -Root $rootDir -Child $buildDir
Assert-ChildPath -Root $rootDir -Child $installDir
Assert-ChildPath -Root $rootDir -Child $envFile

Require-Command "git" | Out-Null
Require-Command "cmake" | Out-Null
Require-Command "pkg-config" | Out-Null

if ($CheckOnly) {
    $pkgConfigDir = Find-PkgConfigDir -InstallDir $installDir
    Add-EnvPath -Name "PKG_CONFIG_PATH" -Value $pkgConfigDir
    Add-EnvPath -Name "PATH" -Value (Join-Path $installDir "bin")
    Add-EnvPath -Name "PATH" -Value (Join-Path $installDir "lib")
    pkg-config --modversion libvvenc
    pkg-config --libs --cflags libvvenc
    Write-Host "VVenC is available through PKG_CONFIG_PATH=$pkgConfigDir"
    exit 0
}

if ($Force) {
    foreach ($path in @($sourceDir, $buildDir, $installDir, $envFile)) {
        if (Test-Path $path) {
            Assert-ChildPath -Root $rootDir -Child $path
            Remove-Item -LiteralPath $path -Recurse -Force
        }
    }
}

if (-not (Test-Path (Join-Path $sourceDir ".git"))) {
    if (Test-Path $sourceDir) {
        throw "Source directory exists but is not a git checkout: $sourceDir. Re-run with -Force to recreate it."
    }
    git clone --depth 1 --branch $Version https://github.com/fraunhoferhhi/vvenc.git $sourceDir
} else {
    git -C $sourceDir fetch --tags --depth 1 origin $Version
    git -C $sourceDir checkout --detach $Version
}

$selectedGenerator = Select-CMakeGenerator -Requested $Generator
$configureArgs = @(
    "-S", $sourceDir,
    "-B", $buildDir,
    "-G", $selectedGenerator,
    "-DCMAKE_INSTALL_PREFIX=$installDir"
)

if ($selectedGenerator -like "Visual Studio*") {
    $configureArgs += @("-A", $Architecture)
} else {
    $configureArgs += "-DCMAKE_BUILD_TYPE=$Configuration"
}

cmake @configureArgs

$buildArgs = @(
    "--build", $buildDir,
    "--config", $Configuration,
    "--target", "install"
)
if ($Jobs -gt 0) {
    $buildArgs += @("--parallel", "$Jobs")
} else {
    $buildArgs += "--parallel"
}
cmake @buildArgs

$pkgConfigDir = Find-PkgConfigDir -InstallDir $installDir
Add-EnvPath -Name "PKG_CONFIG_PATH" -Value $pkgConfigDir
Add-EnvPath -Name "PATH" -Value (Join-Path $installDir "bin")
Add-EnvPath -Name "PATH" -Value (Join-Path $installDir "lib")

$versionText = pkg-config --modversion libvvenc
$flagsText = pkg-config --libs --cflags libvvenc
Write-EnvFile -EnvFile $envFile -PkgConfigDir $pkgConfigDir -InstallDir $installDir

Write-Host "Built VVenC $versionText at $installDir"
Write-Host "pkg-config: $flagsText"
Write-Host "Environment file: $envFile"
