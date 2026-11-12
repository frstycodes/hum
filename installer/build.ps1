# Builds hum.exe and the installer: dist\Hum-Setup-<version>.exe
# Needs NSIS 3 (makensis): on PATH, in Program Files, in electron-builder's cache, or
# passed with -MakeNsis.
param([string]$MakeNsis)
$ErrorActionPreference = 'Stop'
$root = Split-Path $PSScriptRoot -Parent

$version = (Select-String -Path "$root\Cargo.toml" -Pattern '^version\s*=\s*"([^"]+)"' | Select-Object -First 1).Matches.Groups[1].Value

if (-not $MakeNsis) {
    $found = Get-Command makensis -ErrorAction SilentlyContinue
    $candidates = @(
        $(if ($found) { $found.Source }),
        "${env:ProgramFiles(x86)}\NSIS\makensis.exe",
        "$env:ProgramFiles\NSIS\makensis.exe"
    ) + @(Get-ChildItem "$env:LOCALAPPDATA\electron-builder\Cache\nsis*\*\makensis.exe" -ErrorAction SilentlyContinue | ForEach-Object FullName)
    $MakeNsis = $candidates | Where-Object { $_ -and (Test-Path $_) } | Select-Object -First 1
}
if (-not $MakeNsis) { throw "makensis not found. Install NSIS 3 (winget install NSIS.NSIS) or pass -MakeNsis <path>." }

# The project sits deep enough that the linker trips over long paths in target\;
# build somewhere short unless told otherwise.
if (-not $env:CARGO_TARGET_DIR) { $env:CARGO_TARGET_DIR = "$env:USERPROFILE\.phonemic-target" }
$target = $env:CARGO_TARGET_DIR

Push-Location $root
try {
    # the Android app first: Hum carries it and hands it to phones (skipped, with a
    # note, when the Android tools aren't on this PC)
    if (Test-Path "$env:LOCALAPPDATA\AndroidBuild\sdk\build-tools") {
        & "$root\android\build.ps1" | Out-Null
    } else {
        Write-Warning 'Android tools not found: building Hum without the Android app.'
    }
    # cargo reports progress on stderr; only its exit code means failure
    $ErrorActionPreference = 'Continue'
    cargo build --release 2>&1 | ForEach-Object { "$_" }
    if ($LASTEXITCODE -ne 0) { throw 'cargo build failed' }
    $ErrorActionPreference = 'Stop'
    # build.rs renders the logo into hum.ico; the installer wears it too
    $ico = Get-ChildItem "$target\release\build\hum-*\out\hum.ico" | Sort-Object LastWriteTime -Descending | Select-Object -First 1
    if (-not $ico) { throw 'hum.ico not found in the build output' }
    New-Item -ItemType Directory -Force "$root\dist" | Out-Null
    & $MakeNsis /V2 "/DVERSION=$version" "/DEXE=$target\release\hum.exe" "/DICON=$($ico.FullName)" "$root\installer\hum.nsi"
    if ($LASTEXITCODE -ne 0) { throw 'makensis failed' }
    Get-Item "$root\dist\Hum-Setup-$version.exe"
} finally {
    Pop-Location
}
