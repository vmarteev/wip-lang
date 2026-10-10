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
# directory the scripts build in, `WIP_TARGET_DIR` — the compiler the seed
# builds, the stages — the packages installed and the C cache live in a
# volume of the guest's own, which is why a run is not held up by a bind
# mount, and why a second run is warm. `container volume delete wip-build` (or
# `docker volume rm wip-build`) throws that away.
#
# The guest gets 8 CPUs and 12 GiB by default: the gate spreads its work
# over the CPUs, and the compiler holds a release build's whole module while
# it writes it, some two gigabytes, beside the programs the gate builds;
# WIP_LINUX_CPUS and WIP_LINUX_MEMORY say otherwise.
set -uo pipefail
cd "$(dirname "$0")/.." || exit 1

# Debian 12 with what a build of anything needs — a C compiler and its
# linker, git, python3 — to which the guest adds what Wip needs below;
# WIP_LINUX_IMAGE names another.
image=${WIP_LINUX_IMAGE:-docker.io/library/buildpack-deps:bookworm}
arch=${WIP_LINUX_ARCH:-}
cpus=${WIP_LINUX_CPUS:-8}
memory=${WIP_LINUX_MEMORY:-12g}
# One volume per architecture: a build is kept in the same paths whatever
# the machine is, so arm64 and x86-64 builds would clobber each other and
# be made again on every switch.
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

# Every build is LLVM's, through a clang 15 or newer, and Debian 12's own
# `clang` is 14: the guest is given `clang-19`, its packages kept in the
# volume, which keeps the image an ordinary one rather than something to
# build; and gdb, which runs a program to show its values through `wip
# debug`'s script, as a Mac lets only a developer's lldb do. Git is told
# the mounted checkout is safe to read, which it doubts, as another user's.
inside='git config --global --add safe.directory /repo
if ! command -v clang-19 >/dev/null || ! command -v gdb >/dev/null; then
    mkdir -p /work/apt/partial
    { apt-get update -qq && apt-get install -y -qq -o Dir::Cache::Archives=/work/apt clang-19 gdb; } >/dev/null 2>&1 ||
        echo "note: clang-19 and gdb could not be installed; nothing is built here without a clang" >&2
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
    -e WIP_TARGET_DIR=/work/target \
    -e WIP_CACHE_DIR=/work/wip-cache \
    -e WIP_LINUX_RUN="$run" \
    "$image" \
    bash -c "$inside" verify "$@"
status=$?
took=$((SECONDS - started))
printf '%-34s %dm%02ds\n' "on Linux, with the container" $((took / 60)) $((took % 60))
exit $status
