# The gate on Windows, as `scripts/verify.sh` is a Unix's, step for step and
# in its words: the seed's compiler, built by `scripts\bootstrap.ps1` and
# kept; stage 1, the compiler built by it; stage 2, built by stage 1, the
# two writing the same bitcode, and stage 1 warning of nothing as it builds
# stage 2, since a warning there is one in the source — on Windows, in code
# no other system compiles; the gate built; `wip fmt`; Unicode's tables;
# the powers of ten, where there is Python; and the gate's checks on stage
# 2, every one, or those named, but iOS's, which needs a Mac, and the
# debugger's.
# The editors' grammar and the citations are bash's, and the same answer
# everywhere: a Unix checks them. Prints ALL OK when they all pass.
#
#   powershell -File scripts\verify.ps1 [CHECK…]
#
# `scripts/windows.sh gate` runs it on a Windows machine from a Mac, and CI
# on GitHub's Windows runners, arm64 and x86-64.
param([Parameter(ValueFromRemainingArguments = $true)][string[]]$Checks = @())
$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'
Set-Location (Join-Path $PSScriptRoot '..')
$started = Get-Date
$failed = $false
$out = Join-Path ([System.IO.Path]::GetTempPath()) "wip-verify-$PID"
New-Item -ItemType Directory -Force $out | Out-Null

function Duration([int]$s) { if ($s -ge 60) { '{0}m{1:d2}s' -f [int][math]::Floor($s / 60), ($s % 60) } else { "${s}s" } }
function Say($name, $how, $since) {
	$took = if ($since) { Duration ((Get-Date) - $since).TotalSeconds } else { '' }
	'{0,-34} {1,-8} {2}' -f $name, $how, $took
}
# A command, its output into `$log`, and whether it succeeded: what it
# writes on its stderr is its words, which PowerShell would stop at. The
# log is written even where the command says nothing, which `Set-Content`
# at the end of a pipeline given nothing does not do.
function Ran($tool, [string[]]$arguments, $log) {
	$ErrorActionPreference = 'Continue'
	$said = @(& $tool @arguments *>&1 | ForEach-Object { "$_" })
	$succeeded = $LASTEXITCODE -eq 0
	[System.IO.File]::WriteAllLines($log, [string[]]$said)
	return $succeeded
}

# Visual Studio's `clang`, where none is named: one on the `Path`, as
# GitHub's runners have LLVM's own, is not the one Wip is built with there.
if (-not $env:WIP_CLANG) {
	$vswhere = Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio\Installer\vswhere.exe'
	$vs = & $vswhere -latest -products * -requires Microsoft.VisualStudio.Component.VC.Llvm.Clang -property installationPath
	if (-not $vs) { throw "no Visual Studio with its Clang is installed: its Build Tools, with the C++ tools and Clang, have one" }
	$hostDir = if ($env:PROCESSOR_ARCHITECTURE -eq 'ARM64') { 'ARM64' } else { 'x64' }
	$env:WIP_CLANG = Join-Path $vs "VC\Tools\Llvm\$hostDir\bin\clang.exe"
}
if (-not $env:WIP_CACHE_DIR) { $env:WIP_CACHE_DIR = Join-Path (Get-Location) 'target\wip-cache' }
if (-not $env:WIP_CHECK_MOVES) { $env:WIP_CHECK_MOVES = '1' }

$step = Get-Date
if (Ran 'powershell' @('-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', 'scripts\bootstrap.ps1', '-Kept') "$out\bootstrap") {
	Say "the seed's compiler" 'ok' $step
} else {
	Say "the seed's compiler" 'FAILED' $step; Get-Content "$out\bootstrap"; exit 1
}
$env:WIP_HOME = (Get-Location).Path

# A stage: the compiler built by `$with` into `$dir`, laid out as an
# installed one, as `bootstrap.ps1` lays out its own.
function Stage($with, $dir, $log) {
	New-Item -ItemType Directory -Force "$dir\bin" | Out-Null
	foreach ($part in 'std', 'tools') {
		if (Test-Path "$dir\$part") { (Get-Item "$dir\$part").Delete() }
		New-Item -ItemType Junction -Path "$dir\$part" -Target (Resolve-Path $part) | Out-Null
	}
	return Ran $with @('build', '--release', 'compiler\main.wip', '-o', "$dir\bin\wip.exe") $log
}
$one = 'target\stages\1'
$two = 'target\stages\2'
$step = Get-Date
if (Stage 'target\bootstrap\bin\wip.exe' $one "$out\stage1") {
	Say 'the compiler, built (stage 1)' 'ok' $step
} else {
	Say 'the compiler, built (stage 1)' 'FAILED' $step; Get-Content "$out\stage1"; exit 1
}

# Stage 2, and the two writing the same bitcode for the compiler, every part.
$step = Get-Date
$same = (Stage "$one\bin\wip.exe" $two "$out\stage2") -and
	(Ran "$one\bin\wip.exe" @('build', '--release', '--emit', 'llvm-bc', 'compiler\main.wip', '-o', "$out\1\wip.bc") "$out\bitcode1") -and
	(Ran "$two\bin\wip.exe" @('build', '--release', '--emit', 'llvm-bc', 'compiler\main.wip', '-o', "$out\2\wip.bc") "$out\bitcode2")
if ($same) {
	$first = @(Get-ChildItem "$out\1"); $second = @(Get-ChildItem "$out\2")
	$same = $first.Count -eq $second.Count
	foreach ($part in $first) {
		$other = Join-Path "$out\2" $part.Name
		if (-not (Test-Path $other) -or (Get-FileHash $part.FullName).Hash -ne (Get-FileHash $other).Hash) { $same = $false }
	}
}
if ($same) {
	Say 'the compiler, built by itself' 'ok' $step
} else {
	Say 'the compiler, built by itself' 'FAILED' $step
	foreach ($log in 'stage2', 'bitcode1', 'bitcode2') { if (Test-Path "$out\$log") { Get-Content "$out\$log" | Select-Object -Last 20 } }
	exit 1
}
$wip = "$two\bin\wip.exe"

if (Select-String -Quiet -Pattern '\] Warning:' "$out\stage2") {
	Say 'the compiler, without warnings' 'FAILED' $null; Get-Content "$out\stage2"; $failed = $true
} else {
	Say 'the compiler, without warnings' 'ok' $null
}

$step = Get-Date
if (Ran $wip @('build', '--release', 'tests\gate\main.wip', '-o', "$out\gate.exe") "$out\gate-build") {
	Say 'the gate, built' 'ok' $step
} else {
	Say 'the gate, built' 'FAILED' $step; Get-Content "$out\gate-build"; $failed = $true
}

$step = Get-Date
if (Ran $wip @('fmt', '--check', 'std', 'tests\std', 'tests\gate', 'tests\fuzz', 'examples', 'tools\bindgen', 'tools\unicode', 'compiler') "$out\wip-fmt") {
	Say 'wip fmt' 'ok' $step
} else {
	Say 'wip fmt' 'FAILED' $step; Get-Content "$out\wip-fmt"; $failed = $true
}

$python = Get-Command python, python3 -ErrorAction SilentlyContinue | Select-Object -First 1
$step = Get-Date
if (-not $python) {
	Say 'powers of ten' 'skipped: no Python' $null
} elseif (Ran $python.Source @('scripts\float-powers.py', '--check') "$out\powers") {
	Say 'powers of ten' 'ok' $step
} else {
	Say 'powers of ten' 'FAILED' $step; Get-Content "$out\powers"; $failed = $true
}
$step = Get-Date
if (Ran $wip @('run', 'tools\unicode\main.wip', '--check') "$out\unicode") {
	Say "Unicode's tables" 'ok' $step
} else {
	Say "Unicode's tables" 'FAILED' $step; Get-Content "$out\unicode"; $failed = $true
}
Say "editors' grammar" 'skipped: checked on a Unix' $null
Say 'citations' 'skipped: checked on a Unix' $null

if (Test-Path "$out\gate.exe") {
	$step = Get-Date
	if (Ran "$out\gate.exe" (@($wip, '--except', 'ios', '--except', 'debugger') + $Checks) "$out\checks") {
		Say "the gate's checks" 'ok' $step
		Get-Content "$out\checks" | ForEach-Object { "  $_" }
	} else {
		Say "the gate's checks" 'FAILED' $step; Get-Content "$out\checks"; $failed = $true
	}
}

$total = Duration ((Get-Date) - $started).TotalSeconds
Remove-Item -Recurse -Force $out -ErrorAction SilentlyContinue
if ($failed) { "FAILED in $total"; exit 1 }
"ALL OK in $total"
