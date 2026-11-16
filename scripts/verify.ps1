# The gate on Windows, as `scripts/verify.sh` is a Unix's: `wip` built from
# this system's seed by `scripts\bootstrap.ps1`, its stages compared, and
# the gate built by it and run with it — every check, or those named, but
# iOS's, which needs a Mac, and the debugger's, which drives LLDB.
#
#   powershell -File scripts\verify.ps1 [CHECK…]
#
# `scripts/windows.sh gate` runs it on a Windows machine from a Mac, and CI
# on GitHub's Windows runners, arm64 and x86-64.
param([Parameter(ValueFromRemainingArguments = $true)][string[]]$Checks = @())
$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'
Set-Location (Join-Path $PSScriptRoot '..')

# Visual Studio's `clang`, where none is named: one on the `Path`, as
# GitHub's runners have LLVM's own, is not the one Wip is built with there.
if (-not $env:WIP_CLANG) {
	$vswhere = Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio\Installer\vswhere.exe'
	$vs = & $vswhere -latest -products * -requires Microsoft.VisualStudio.Component.VC.Llvm.Clang -property installationPath
	if (-not $vs) { throw "no Visual Studio with its Clang is installed: its Build Tools, with the C++ tools and Clang, have one" }
	$hostDir = if ($env:PROCESSOR_ARCHITECTURE -eq 'ARM64') { 'ARM64' } else { 'x64' }
	$env:WIP_CLANG = Join-Path $vs "VC\Tools\Llvm\$hostDir\bin\clang.exe"
}

& powershell -NoProfile -ExecutionPolicy Bypass -File scripts\bootstrap.ps1
if ($LASTEXITCODE -ne 0) { exit 1 }
$wip = 'target\bootstrap\bin\wip.exe'
$ErrorActionPreference = 'Continue'
& $wip build --release tests\gate\main.wip -o target\gate.exe 2>&1 | ForEach-Object { "$_" }
if ($LASTEXITCODE -ne 0) { exit 1 }
& target\gate.exe $wip --except ios --except debugger @Checks 2>&1 | ForEach-Object { "$_" }
exit $LASTEXITCODE
