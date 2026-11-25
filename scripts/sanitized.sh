#!/bin/bash
# The compiler under the address sanitizer: built with `--sanitize address`
# into `target/sanitized`, and then the gate's checks of what it compiles
# run with it, and the compiler's own tests built sanitized. A read or a
# write of memory the compiler does not own — freed, past an allocation, or
# a variable's slot after its block — stops it with the sanitizer's report,
# which fails the run. Its leaks are not counted: the compiler keeps what it
# reads until it ends.
#
#   scripts/sanitized.sh                 the gate's checks and the compiler's tests
#   scripts/sanitized.sh --ports         and each port's tests, built sanitized too,
#                                        those `scripts/ports.txt` lists
#   scripts/sanitized.sh --threads       and the library's and the compiler's
#                                        tests under the thread sanitizer
#   scripts/sanitized.sh CHECK…          those checks of the gate alone
#
# The thread sanitizer reports two threads that reach one place, one of
# them writing, with nothing to order them; `scripts/tsan.supp` names what
# it reports that is a system's blind spot and not a race. Its pass is the
# longest, three quarters of an hour, most of it the compiler's tests.
#
# It is slow — the sanitizer makes the compiler several times slower — so
# it runs before a release and after a change to the backend, the drops or
# the library's memory, not before every commit. It writes to
# `target/sanitized.log`, and says at the end whether the sanitizer
# reported anything, and where.
set -uo pipefail
cd "$(dirname "$0")/.."
target=${WIP_TARGET_DIR:-target}
log=$target/sanitized.log
ports=false
threads=false
checks=()
for arg in "$@"; do
    case $arg in
    --ports) ports=true ;;
    --threads) threads=true ;;
    *) checks+=("$arg") ;;
    esac
done
if [ ${#checks[@]} -eq 0 ]; then
    checks=(cases reference examples library packages bindgen lsp driver)
fi

# The compiler's leaks are not its errors; a report is its own exit code,
# so that a refusal, 1, is not taken for one.
export ASAN_OPTIONS=detect_leaks=0:detect_stack_use_after_return=1:exitcode=86

: >"$log"
step() {
    echo "== $*" | tee -a "$log"
}

# The seed's compiler is older than `--sanitize`: the sanitized one is
# built by one built from this checkout.
step "the compiler, and the compiler sanitized"
scripts/build.sh --release >>"$log" 2>&1 || {
    echo "the compiler does not build; see $log"
    exit 1
}
wip=$PWD/$target/wip/bin/wip
scripts/build.sh --release --sanitize address --with "$wip" --out "$target/sanitized" >>"$log" 2>&1 || {
    echo "the sanitized compiler does not build; see $log"
    exit 1
}
sanitized=$PWD/$target/sanitized/bin/wip
"$wip" build --release tests/gate/main.wip -o "$target/gate-sanitized" >>"$log" 2>&1 || {
    echo "the gate does not build; see $log"
    exit 1
}

failed=0
step "the gate's checks, ${checks[*]}, with the sanitized compiler"
"$target/gate-sanitized" "$sanitized" "${checks[@]}" 2>&1 | tee -a "$log" || failed=1

step "the compiler's own tests, built sanitized"
WIP_HOME=$PWD "$wip" test --release --sanitize address compiler/main.wip >>"$log" 2>&1 || failed=1
tail -3 "$log"

if $ports; then
    # Each port's tests, compiled by the sanitized compiler and built
    # sanitized themselves: the port's own memory is checked too. The
    # ports are a checkout's own, listed in `scripts/ports.txt`, a
    # directory and what `wip test` is given in it to a line.
    listed=scripts/ports.txt
    [ -f "$listed" ] || step "no $listed: no ports to test"
    while read -r dir entry <&3; do
        case "$dir" in '' | '#'*) continue ;; esac
        [ "$entry" = - ] && continue
        if [ ! -d "$dir" ]; then
            step "$dir: not here, passed over"
            continue
        fi
        step "$dir: wip test --sanitize address $entry"
        (cd "$dir" && "$sanitized" test --sanitize address "$entry") >>"$log" 2>&1 || {
            failed=1
            echo "  failed; see $log" | tee -a "$log"
        }
    done 3< <([ -f "$listed" ] && cat "$listed")
fi

if $threads; then
    # What a system orders that the sanitizer cannot see it order is named
    # in the suppressions; a report is its own exit code.
    export TSAN_OPTIONS="halt_on_error=0:exitcode=66:suppressions=$PWD/scripts/tsan.supp"
    step "the library's tests, built with the thread sanitizer"
    "$wip" test --sanitize thread tests/std/main.wip >>"$log" 2>&1 || failed=1
    tail -2 "$log"
    step "the compiler's own tests, built with the thread sanitizer"
    WIP_HOME=$PWD "$wip" test --sanitize thread compiler/main.wip >>"$log" 2>&1 || failed=1
    tail -2 "$log"
fi

if grep -q "WARNING: ThreadSanitizer" "$log"; then
    echo "the thread sanitizer reported, in $log:"
    grep -n -A3 "WARNING: ThreadSanitizer" "$log" | head -20
    exit 1
fi
if grep -q "ERROR: AddressSanitizer" "$log"; then
    echo "the sanitizer reported, in $log:"
    grep -n "ERROR: AddressSanitizer" "$log" | head -20
    exit 1
fi
if [ $failed -ne 0 ]; then
    echo "FAILED, with nothing the sanitizer reported; see $log"
    exit 1
fi
echo "ALL OK, with nothing the sanitizer reported"
