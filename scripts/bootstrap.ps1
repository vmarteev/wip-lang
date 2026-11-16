# Builds `wip` from this checkout on Windows, with Visual Studio's `clang`
# and Microsoft's linker alone, as `scripts/bootstrap.sh` does on a Unix.
#
#   powershell -File scripts\bootstrap.ps1 [-Out DIR] [-Kept]
#
# 1. Stage A: this system's seed, `bootstrap\<system>\wip.bc.gz`, compiled
#    by `clang` and linked as its `link.txt` says, with an empty table of
#    the calls a panic lists: a `wip` whose only work is to build the next.
# 2. Stage B: stage A builds the compiler from `compiler\`, a release
#    build.
# 3. Stage C: B builds it again, and B and C must write the same bitcode
#    for the compiler, every part of it.
#
# C is the `wip` it leaves, laid out as an installed compiler is, in `-Out`
# (`target\bootstrap` where none is named): `bin\wip.exe`, and beside it
# the checkout's `std\` and `tools\`, as junctions. `clang` is
# `$env:WIP_CLANG`, or the one Visual Studio's installer says it has: its
# Build Tools, with the C++ tools and Clang, for this machine's processor.
#
# `-Kept` keeps a `wip` it built before, in the same `-Out`, unless the
# seed is newer, as `bootstrap.sh --kept` does.
param([string]$Out = "target\bootstrap", [switch]$Kept)
$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'
Set-Location (Join-Path $PSScriptRoot '..')

$system = switch ($env:PROCESSOR_ARCHITECTURE) {
	'ARM64' { 'windows-arm64' }
	'AMD64' { 'windows-x86_64' }
	default { throw "there is no seed for Windows on $env:PROCESSOR_ARCHITECTURE" }
}
$seed = "bootstrap\$system"
if (-not (Test-Path "$seed\wip.bc.gz")) { throw "there is no seed in $seed" }
if ($Kept -and (Test-Path "$Out\bin\wip.exe") -and
	(Get-Item "$seed\wip.bc.gz").LastWriteTime -le (Get-Item "$Out\bin\wip.exe").LastWriteTime) {
	exit 0
}

$clang = $env:WIP_CLANG
if (-not $clang) {
	$vswhere = Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio\Installer\vswhere.exe'
	$vs = & $vswhere -latest -products * -requires Microsoft.VisualStudio.Component.VC.Llvm.Clang -property installationPath
	if (-not $vs) { throw "no Visual Studio with its Clang is installed: its Build Tools, with the C++ tools and Clang, have one" }
	$hostDir = if ($env:PROCESSOR_ARCHITECTURE -eq 'ARM64') { 'ARM64' } else { 'x64' }
	$clang = Join-Path $vs "VC\Tools\Llvm\$hostDir\bin\clang.exe"
}
$env:WIP_CLANG = $clang

$work = Join-Path $Out 'stages'
New-Item -ItemType Directory -Force $work | Out-Null
$started = Get-Date
function Say($what) { '{0,-34} {1}s' -f $what, [int]((Get-Date) - $started).TotalSeconds }

# A command run, its output kept and said only where it fails: what it
# writes on its stderr is its words, which PowerShell would stop at.
function Run($tool, [string[]]$arguments) {
	$ErrorActionPreference = 'Continue'
	$said = & $tool @arguments 2>&1 | ForEach-Object { "$_" }
	if ($LASTEXITCODE -ne 0) {
		$said | ForEach-Object { [Console]::Error.WriteLine($_) }
		throw "``$tool`` failed"
	}
}

# Stage A. The seed unpacked, and the table of calls a list of none: no
# entries, where they begin and end, and no parts.
$packed = [System.IO.File]::OpenRead((Resolve-Path "$seed\wip.bc.gz"))
$bitcode = [System.IO.File]::Create((Join-Path (Resolve-Path $work) 'seed.bc'))
$unpacking = New-Object System.IO.Compression.GZipStream($packed, [System.IO.Compression.CompressionMode]::Decompress)
$unpacking.CopyTo($bitcode)
$unpacking.Dispose(); $bitcode.Dispose(); $packed.Dispose()
Set-Content -Encoding ascii "$work\tables.ll" '@"wip.frame_tables" = hidden global [3 x i64] zeroinitializer, align 8'
Run $clang @('-O2', '-Wno-override-module', '-c', "$work\seed.bc", '-o', "$work\seed.o")
Run $clang @('-Wno-override-module', '-c', "$work\tables.ll", '-o', "$work\tables.o")
$link = @(Get-Content "$seed\link.txt" | Where-Object { $_ -ne '' })
New-Item -ItemType Directory -Force "$work\A\bin" | Out-Null
Run $clang (@('-fuse-ld=link', '-o', "$work\A\bin\wip.exe", "$work\seed.o", "$work\tables.o") + $link)
Say 'stage A, from the seed'

# The commit checked out and its day, which the compiler embeds, so that
# `wip --version` names them; a copy with no history writes none.
$commit = 'compiler\lang\commit.txt'
$hash = $null
if (Get-Command git -ErrorAction SilentlyContinue) {
	$ErrorActionPreference = 'Continue'
	$hash = git --no-pager rev-parse --short=12 HEAD 2>$null
	$day = git --no-pager log -1 --format=%cs HEAD 2>$null
	$ErrorActionPreference = 'Stop'
}
if ($hash -and $day) {
	$text = "$hash $day"
	if (-not (Test-Path $commit) -or (Get-Content -Raw $commit) -ne $text) {
		[System.IO.File]::WriteAllText((Join-Path (Get-Location) $commit), $text)
	}
} elseif (Test-Path $commit) {
	Remove-Item $commit
}

# A compiler built by `$with` into `$dir`, laid out as an installed one,
# its home the checkout's library and tools, as junctions.
$env:WIP_HOME = (Get-Location).Path
function Built($with, $dir) {
	New-Item -ItemType Directory -Force "$dir\bin" | Out-Null
	foreach ($part in 'std', 'tools') {
		if (Test-Path "$dir\$part") { (Get-Item "$dir\$part").Delete() }
		New-Item -ItemType Junction -Path "$dir\$part" -Target (Resolve-Path $part) | Out-Null
	}
	Run $with @('build', '--release', 'compiler\main.wip', '-o', "$dir\bin\wip.exe")
}
Built "$work\A\bin\wip.exe" "$work\B"
Say 'stage B, built by A'
Built "$work\B\bin\wip.exe" $Out
Say 'stage C, built by B'

# B and C write the same bitcode for the compiler, every part of it.
foreach ($stage in 'B', 'C') {
	if (Test-Path "$work\$stage-bc") { Remove-Item -Recurse -Force "$work\$stage-bc" }
	New-Item -ItemType Directory -Force "$work\$stage-bc" | Out-Null
}
Run "$work\B\bin\wip.exe" @('build', '--release', '--emit', 'llvm-bc', 'compiler\main.wip', '-o', "$work\B-bc\wip.bc")
Run "$Out\bin\wip.exe" @('build', '--release', '--emit', 'llvm-bc', 'compiler\main.wip', '-o', "$work\C-bc\wip.bc")
foreach ($part in Get-ChildItem "$work\B-bc") {
	$other = Join-Path "$work\C-bc" $part.Name
	if (-not (Test-Path $other) -or (Get-FileHash $part.FullName).Hash -ne (Get-FileHash $other).Hash) {
		throw "stages B and C write different bitcode for the compiler: $($part.Name)"
	}
}
if ((Get-ChildItem "$work\C-bc").Count -ne (Get-ChildItem "$work\B-bc").Count) {
	throw "stages B and C write the compiler in different parts"
}
Say 'B and C write the same bitcode'
"wip is $Out\bin\wip.exe: $(& "$Out\bin\wip.exe" --version)"
