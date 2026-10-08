#!/bin/bash
# Runs the verification on Linux, in a container, from a machine that is not
# Linux. Wip is developed on Apple arm64, and the differences that bite are
# the ones a second platform finds: the C ABI, the size of `c_long`, what
# the linker wants, and the `.m` files that are Apple's alone.
#
#   scripts/linux.sh             everything, as scripts/verify.sh does
#   scripts/linux.sh --fast      the language's own cases, and nothing else
#
# WIP_LINUX_RUN names another script of the repository's to run in the
# guest in verify.sh's place, with the same arguments: scripts/clangs.sh
# runs itself there so.
#
# It runs under Apple's `container` where that is installed, and under
# `docker` otherwise; WIP_CONTAINER names one directly. The image is
# WIP_LINUX_IMAGE, and WIP_LINUX_ARCH=amd64 asks for an x86-64 guest
# instead of the arm64 one, which on Apple silicon runs through Rosetta
# and is slow but is the other ABI.
#
# What Linux does not cover: anything about how macOS links.
#
# The repository is mounted read-write, but nothing is built into it: the
# target directory, cargo's downloads and the C cache live in a volume of
# the guest's own, which is why a run is not held up by a bind mount, and
# why a second run is warm. `container volume delete wip-build` (or
# `docker volume rm wip-build`) throws that away.
#
# The guest gets 8 CPUs and 8 GiB by default, since a Rust build wants
# both; WIP_LINUX_CPUS and WIP_LINUX_MEMORY say otherwise.
set -uo pipefail
cd "$(dirname "$0")/.." || exit 1

# The image carries the toolchain this machine has, so that a lint or a
# warning tells the same story on both sides; WIP_LINUX_IMAGE overrides it.
host_rust=$(rustc --version 2>/dev/null | cut -d' ' -f2 | cut -d. -f1,2)
image=${WIP_LINUX_IMAGE:-docker.io/library/rust:${host_rust:-stable}-bookworm}
arch=${WIP_LINUX_ARCH:-}
cpus=${WIP_LINUX_CPUS:-8}
memory=${WIP_LINUX_MEMORY:-8g}
# One volume per architecture: cargo keeps a host build in the same paths
# whatever the machine is, so arm64 and x86-64 artifacts would clobber
# each other and rebuild on every switch.
volume=${WIP_LINUX_VOLUME:-wip-build${arch:+-$arch}}

# Apple's `container` (github.com/apple/container) runs Linux in a
# lightweight VM and takes the same flags this needs; Docker is the
# fallback. Both are asked whether they are actually up.
runtime=${WIP_CONTAINER:-}
if [ -z "$runtime" ]; then
    if command -v container >/dev/null 2>&1 && container system status >/dev/null 2>&1; then
        runtime=container
    elif command -v docker >/dev/null 2>&1 && docker info >/dev/null 2>&1; then
        runtime=docker
    fi
fi

case "$runtime" in
    "")
        echo "error: no container runtime is running" >&2
        echo "  Apple's:  container system start" >&2
        echo "  Docker's: start Docker Desktop" >&2
        exit 2
        ;;
    container)
        if ! container system status >/dev/null 2>&1; then
            echo "error: the container service is not running — container system start" >&2
            exit 2
        fi
        ;;
    docker)
        if ! docker info >/dev/null 2>&1; then
            echo "error: the docker daemon is not running — start Docker Desktop" >&2
            exit 2
        fi
        ;;
esac

# An x86-64 guest is asked for differently by each, and on Apple silicon
# it needs Rosetta to run at all. The array is expanded with the `+`
# idiom, since macOS ships bash 3, where an empty one is "unbound".
platform=()
if [ -n "$arch" ]; then
    case "$runtime" in
        container)
            platform=(--arch "$arch")
            [ "$arch" = "amd64" ] && platform+=(--rosetta)
            ;;
        docker) platform=(--platform "linux/$arch") ;;
    esac
fi

# The official `rust` image carries a minimal toolchain, so the two
# components the gate needs are added first; it takes a couple of seconds
# and keeps the image an ordinary one rather than something to build.
# A release build is LLVM's where there is a clang 15 or newer, and Debian
# 12's own `clang` is 14: the guest is given `clang-19`, its packages kept
# in the volume, so that both backends are checked here as on a Mac; and
# gdb, which runs a program to show its values through `wip debug`'s script,
# as a Mac lets only a developer's lldb do.
inside='command -v rustup >/dev/null && rustup component add rustfmt clippy >/dev/null 2>&1
if ! command -v clang-19 >/dev/null || ! command -v gdb >/dev/null; then
    mkdir -p /work/apt/partial
    { apt-get update -qq && apt-get install -y -qq -o Dir::Cache::Archives=/work/apt clang-19 gdb; } >/dev/null 2>&1 ||
        echo "note: clang-19 and gdb could not be installed; the release builds here are Cranelift'"'"'s" >&2
fi
export WIP_CLANG=clang-19
printf "%-34s %-8s %ss\\n" "the guest made ready" "ok" "$SECONDS"
exec ./"$WIP_LINUX_RUN" "$@"'
run=${WIP_LINUX_RUN:-scripts/verify.sh}
if [ ! -x "$run" ]; then
    echo "error: $run is not a script of the repository's to run" >&2
    exit 2
fi

# The volume attaches to one guest at a time, and a second run that tries
# fails deep in the virtual machine ("the storage device attachment is
# invalid"), so runs are kept one at a time here, where it can be said.
lock=target/.linux-lock
mkdir -p target
if ! mkdir "$lock" 2>/dev/null; then
    echo "error: another Linux run holds $lock — wait for it, or remove the directory if none is running" >&2
    exit 2
fi
trap 'rmdir "$lock" 2>/dev/null' EXIT

# Docker makes a named volume on first use; Apple's `container` wants it
# to exist.
if [ "$runtime" = "container" ] && ! container volume inspect "$volume" >/dev/null 2>&1; then
    container volume create "$volume" >/dev/null || exit 1
fi

echo "verifying on $image (${arch:-arm64}, $runtime, ${cpus} cpus, ${memory})"
# How long it took with the container around it: the guest says how long
# it took to be made ready, and the run what each step took.
started=$SECONDS
"$runtime" run --rm -t \
    ${platform[@]+"${platform[@]}"} \
    -c "$cpus" \
    -m "$memory" \
    -v "$PWD:/repo" \
    -v "$volume:/work" \
    -w /repo \
    -e CARGO_TARGET_DIR=/work/target \
    -e CARGO_HOME=/work/cargo \
    -e WIP_CACHE_DIR=/work/wip-cache \
    -e WIP_LINUX_RUN="$run" \
    "$image" \
    bash -c "$inside" verify "$@"
status=$?
took=$((SECONDS - started))
printf '%-34s %dm%02ds\n' "on Linux, with the container" $((took / 60)) $((took % 60))
exit $status
