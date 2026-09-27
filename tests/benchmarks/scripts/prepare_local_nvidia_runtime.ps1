param(
  [string]$DriverDirectory = "",
  [string]$RuntimeDirectory = ""
)

# Prepare DLL aliases for this PowerShell process and its child processes.
# Uses the installed display driver's files; does not install drivers or change
# machine/user PATH. Run before cargo or the service in the same shell.
$ErrorActionPreference = 'Stop'
if (-not [Environment]::Is64BitProcess) {
  throw 'The local NVIDIA runtime requires a 64-bit PowerShell process.'
}

$repo = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '../../..'))
if ([string]::IsNullOrWhiteSpace($RuntimeDirectory)) {
  $RuntimeDirectory = Join-Path $repo 'artifacts/local-zero-copy/nvidia-runtime'
}

if ([string]::IsNullOrWhiteSpace($DriverDirectory)) {
  $adapters = Get-ItemProperty 'HKLM:/SYSTEM/CurrentControlSet/Control/Class/{4d36e968-e325-11ce-bfc1-08002be10318}/*' -ErrorAction SilentlyContinue |
    Where-Object { $_.DriverDesc -like '*NVIDIA*' }
  $directories = @($adapters | ForEach-Object {
    @($_.OpenGLDriverName) | ForEach-Object {
      if (-not [string]::IsNullOrWhiteSpace($_) -and (Test-Path -LiteralPath $_ -PathType Leaf)) {
        Split-Path -Parent $_
      }
    }
  } | Sort-Object -Unique)
  if ($directories.Count -ne 1) {
    throw 'Cannot identify a unique installed NVIDIA driver directory; provide -DriverDirectory.'
  }
  $DriverDirectory = $directories[0]
}

$driver = (Resolve-Path -LiteralPath $DriverDirectory).Path
$aliases = [ordered]@{
  'nvcuda.dll' = @('nvcuda.dll', 'nvcuda_loader64.dll')
  'nvcuvid.dll' = @('nvcuvid.dll', 'nvcuvid64.dll')
  'nvEncodeAPI64.dll' = @('nvEncodeAPI64.dll')
  'nvml.dll' = @('nvml.dll')
}

# Resolve every source before changing the output directory or environment.
$sources = @{}
foreach ($alias in $aliases.Keys) {
  $source = $aliases[$alias] | ForEach-Object { Join-Path $driver $_ } |
    Where-Object { Test-Path -LiteralPath $_ -PathType Leaf } | Select-Object -First 1
  if (-not $source) { throw "Installed driver is missing the source for $alias in $driver" }
  $sources[$alias] = $source
}

$runtime = [IO.Path]::GetFullPath($RuntimeDirectory)
if ($runtime.TrimEnd('\') -eq $driver.TrimEnd('\') -or
    $runtime.StartsWith($env:WINDIR.TrimEnd('\') + '\', [StringComparison]::OrdinalIgnoreCase)) {
  throw 'RuntimeDirectory must be a separate writable directory outside the Windows installation.'
}
New-Item -ItemType Directory -Path $runtime -Force | Out-Null
$files = foreach ($alias in $aliases.Keys) {
  $source = $sources[$alias]
  $destination = Join-Path $runtime $alias
  if (-not (Test-Path -LiteralPath $destination) -or
      (Get-FileHash -LiteralPath $destination).Hash -ne (Get-FileHash -LiteralPath $source).Hash) {
    Copy-Item -LiteralPath $source -Destination $destination -Force
  }
  [pscustomobject]@{
    alias = $alias
    source = $source
    version = (Get-Item -LiteralPath $source).VersionInfo.FileVersion
    sha256 = (Get-FileHash -LiteralPath $destination).Hash
  }
}

$manifest = [pscustomobject]@{
  created_at = [DateTimeOffset]::Now.ToString('o')
  driver_directory = $driver
  runtime_directory = $runtime
  environment_scope = 'current process and children only'
  files = @($files)
}
$manifest | ConvertTo-Json -Depth 5 | Set-Content -LiteralPath (Join-Path $runtime 'manifest.json') -Encoding UTF8
if (@($env:PATH -split ';') -notcontains $runtime) {
  $env:PATH = $runtime + ';' + $env:PATH
}
Write-Output "Prepared local NVIDIA runtime: $runtime"
Write-Output 'Run tests or start mrd-service from this shell to inherit the DLL search path.'
