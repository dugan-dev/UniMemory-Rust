param([string]$VcpkgRoot = $env:VCPKG_ROOT)
$ErrorActionPreference = 'Stop'
if (-not $VcpkgRoot) { throw 'Set VCPKG_ROOT to a bootstrapped vcpkg checkout.' }
$root = Split-Path $PSScriptRoot -Parent
$arguments = @(
    'install', 'jemalloc:x64-windows-static-md', '--classic',
    "--overlay-triplets=$PSScriptRoot/triplets",
    "--x-install-root=$root/artifacts/native-sdk",
    "--x-buildtrees-root=$root/artifacts/native-buildtrees",
    "--x-packages-root=$root/artifacts/native-packages",
    '--no-binarycaching'
)
& "$VcpkgRoot/vcpkg.exe" @arguments
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
Write-Output "SDK ready: $root/artifacts/native-sdk/x64-windows-static-md"
