#!/bin/bash
# Builds a program for Windows here and runs it on a Windows machine, over
# SSH: its objects are written by this checkout's `wip` with `--emit
# objects`, copied there, linked with Visual Studio's tools — `clang`,
# Microsoft's linker and the Windows SDK — and run, its output and its exit
# code this script's.
#
#   scripts/windows.sh run PROGRAM.wip [ARGUMENT…]
#   scripts/windows.sh run --release PROGRAM.wip [ARGUMENT…]
#   scripts/windows.sh test FILE.wip [FILTER]    its tests, as `wip test` runs them,
#                                                beside a copy of the library
#   scripts/windows.sh gate [CHECK…]             the gate there, with `wip` built
#                                                there from the seed
#
# WIP_WINDOWS_HOST is the machine, as `ssh` names it (`user@host`), and
# WIP_WINDOWS_KEY the key it is reached with, where `ssh` is not told
# otherwise. The machine runs OpenSSH's server, with PowerShell, and has
# Visual Studio's Build Tools with the C++ tools for its processor, Clang
# and a Windows SDK. WIP_WINDOWS_ARCH is `arm64`, the default, or `x86_64`.
# WIP names the `wip` that builds, `target/wip/bin/wip` by default.
set -uo pipefail
cd "$(dirname "$0")/.." || exit 1

usage() {
    echo "usage: scripts/windows.sh run [--release] PROGRAM.wip [ARGUMENT…]" >&2
    echo "       scripts/windows.sh test [--release] FILE.wip [FILTER]" >&2
    echo "       scripts/windows.sh gate [CHECK…]" >&2
    exit 2
}
case "${1:-}" in
gate) ;;
run | test) [ $# -ge 2 ] || usage ;;
*) usage ;;
esac
mode=$1
shift
profile=()
program=""
if [ "$mode" != gate ]; then
    if [ "$1" = --release ]; then
        profile=(--release)
        shift
    fi
    [ $# -ge 1 ] || usage
    program=$1
    shift
fi
# What the tests are filtered by is the build's, and the runner is given
# nothing.
filter=()
if [ "$mode" = test ]; then
    [ $# -le 1 ] || usage
    filter=("$@")
    set --
fi

host=${WIP_WINDOWS_HOST:-}
if [ -z "$host" ]; then
    echo "error: WIP_WINDOWS_HOST names no Windows machine: user@host, as ssh reaches it" >&2
    exit 2
fi
ssh_args=(-o BatchMode=yes -o ConnectTimeout=10)
if [ -n "${WIP_WINDOWS_KEY:-}" ]; then
    ssh_args+=(-i "$WIP_WINDOWS_KEY" -o IdentitiesOnly=yes)
fi
arch=${WIP_WINDOWS_ARCH:-arm64}
case "$arch" in
    arm64) triple=aarch64-pc-windows-msvc; host_dir=ARM64 ;;
    x86_64) triple=x86_64-pc-windows-msvc; host_dir=x64 ;;
    *) echo "error: WIP_WINDOWS_ARCH is arm64 or x86_64, not $arch" >&2; exit 2 ;;
esac
wip=${WIP:-target/wip/bin/wip}
export WIP_HOME=${WIP_HOME:-$PWD}


name=$(basename "$program" .wip)
[ "$name" = main ] && name=$(basename "$(dirname "$program")")
objects="target/windows/$name"
mkdir -p target/windows
if [ "$mode" = test ]; then
    built=("$wip" test ${profile[@]+"${profile[@]}"} --target "windows-$arch" --emit objects "$program" ${filter[@]+"${filter[@]}"} -o "$objects")
elif [ "$mode" = run ]; then
    built=("$wip" build ${profile[@]+"${profile[@]}"} --target "windows-$arch" --emit objects "$program" -o "$objects")
fi
if [ "$mode" != gate ] && ! "${built[@]}"; then
    exit 1
fi

# PowerShell, run there, given as UTF-16 in base64: `cmd`, which OpenSSH
# runs a command in, would read its quotes.
remote() {
    local encoded
    encoded=$(printf '%s' "$1" | iconv -f UTF-8 -t UTF-16LE | base64 | tr -d '\n')
    ssh "${ssh_args[@]}" "$host" "powershell -NoProfile -NonInteractive -EncodedCommand $encoded" \
        2> >(grep -v -e "post-quantum" -e "store now" -e "may need to be upgraded" >&2)
}

# The gate there: the checkout copied — what git keeps, as it is here —
# into one kept there, whose `target\` outlives a run so that the next is
# warm, and `scripts\verify.ps1` run in it, as CI runs it on GitHub's
# Windows runners.
if [ "$mode" = gate ]; then
    dir="wip-windows/checkout"
    remote "\$ProgressPreference = 'SilentlyContinue'; New-Item -ItemType Directory -Force \"\$env:USERPROFILE\\$dir\" | Out-Null; Get-ChildItem \"\$env:USERPROFILE\\$dir\" -Exclude target | Remove-Item -Recurse -Force" || exit 1
    git ls-files -z --cached --others --exclude-standard |
        COPYFILE_DISABLE=1 tar --null -T - -cf - |
        ssh "${ssh_args[@]}" "$host" "cd $dir && tar -xf -" \
            2> >(grep -v -e "post-quantum" -e "store now" -e "may need to be upgraded" >&2) || exit 1
    checks=""
    for check in "$@"; do
        checks+=" '$check'"
    done
    remote "
\$ProgressPreference = 'SilentlyContinue'
Set-Location \"\$env:USERPROFILE\\$dir\"
& powershell -NoProfile -ExecutionPolicy Bypass -File scripts\\verify.ps1$checks 2>&1 | ForEach-Object { \"\$_\" }
exit \$LASTEXITCODE
"
    exit $?
fi

dir="wip-windows/$name"
remote "\$ProgressPreference = 'SilentlyContinue'; Remove-Item -Recurse -Force -ErrorAction SilentlyContinue \"\$env:USERPROFILE\\$dir\"; New-Item -ItemType Directory -Force \"\$env:USERPROFILE\\$dir\" | Out-Null" || exit 1
# What `link.txt` names, and itself: the directory may hold objects of an
# earlier build.
files=("$objects/link.txt")
while read -r kind what; do
    if [ "$kind" = object ]; then
        files+=("$objects/$what")
    fi
done < "$objects/link.txt"
scp -q "${ssh_args[@]}" "${files[@]}" "$host:$dir/" 2> >(grep -v -e "post-quantum" -e "store now" -e "may need to be upgraded" >&2) || exit 1

# The library's tests read the checkout they are run from, and write into
# its `target/`: a test runs there beside a copy of the library and its
# tests, as `tar` carries them, Windows' own among its commands.
if [ "$mode" = test ]; then
    COPYFILE_DISABLE=1 tar -cf - std tests/std |
        ssh "${ssh_args[@]}" "$host" "cd $dir && tar -xf - && mkdir target" \
            2> >(grep -v -e "post-quantum" -e "store now" -e "may need to be upgraded" >&2) || exit 1
fi

# Each argument quoted for PowerShell: in single quotes, a quote doubled.
quote="'"
quoted=""
for arg in "$@"; do
    quoted+=" '${arg//$quote/$quote$quote}'"
done

remote "
\$ProgressPreference = 'SilentlyContinue'
\$ErrorActionPreference = 'Stop'
Set-Location \"\$env:USERPROFILE\\$dir\"
\$vswhere = Join-Path \${env:ProgramFiles(x86)} 'Microsoft Visual Studio\\Installer\\vswhere.exe'
\$vs = & \$vswhere -products * -latest -requires Microsoft.VisualStudio.Component.VC.Llvm.Clang -property installationPath
if (-not \$vs) { [Console]::Error.WriteLine('error: no Visual Studio with Clang is installed there'); exit 2 }
\$clang = Join-Path \$vs 'VC\\Tools\\Llvm\\$host_dir\\bin\\clang.exe'
\$link = @()
\$parts = @()
foreach (\$line in Get-Content link.txt) {
    \$kind, \$what = \$line -split ' ', 2
    if (\$kind -eq 'object') { \$parts += \$what }
    if (\$kind -eq 'library') { \$link += \"-l\$what\" }
    if (\$kind -eq 'runtime') { \$link += \"--rtlib=\$what\" }
    if (\$kind -eq 'option') { \$link += \"-Wl,\$what\" }
}
# Clang's messages come on its stderr, which PowerShell reads as errors and
# would stop at the first of.
\$ErrorActionPreference = 'Continue'
\$out = & \$clang --target=$triple -fuse-ld=link @parts @link -o '$name.exe' 2>&1
if (\$LASTEXITCODE -ne 0) { \$out | ForEach-Object { [Console]::Error.WriteLine(\"\$_\") }; exit 1 }
& '.\\$name.exe'$quoted
exit \$LASTEXITCODE
"
