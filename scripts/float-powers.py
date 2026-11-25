#!/usr/bin/env python3
# Writes std/prelude/float_powers.wip: the powers of ten a float is read
# with, each to 128 bits, from Python's integers, which are exact.
#
#   scripts/float-powers.py          write it
#   scripts/float-powers.py --check  fail if it is not what would be written
#
# 10^q is written as a 128-bit integer times a power of two, the integer
# from 2^127 up to 2^128, its fraction cut off: exactly 10^q for q from 0 to
# 55, whose powers of five fit, and less than it by under one otherwise.
# Its power of two is floor(q log2 10) - 127, which the library works out
# as `(q × 217706) >> 16`; that is checked here for each q.
import sys

LEAST = -342
MOST = 308


def power(q):
    """10^q's 128 bits, the fraction cut off, and its power of two."""
    if q >= 0:
        value = 10**q
        bits = value.bit_length()
        if bits > 128:
            return value >> (bits - 128), bits - 128
        return value << (128 - bits), bits - 128
    # 2^k / 10^-q, for the least k that makes it 128 bits.
    divisor = 10**-q
    k = divisor.bit_length() + 127
    top = (1 << k) // divisor
    if top.bit_length() < 128:
        k += 1
        top = (1 << k) // divisor
    assert top.bit_length() == 128
    return top, -k


def text():
    rows = []
    for q in range(LEAST, MOST + 1):
        top, exponent = power(q)
        assert exponent == ((q * 217706) >> 16) - 127, q
        rows.append(f"\t0x{top >> 64:016X}, 0x{top & ((1 << 64) - 1):016X},")
    return f"""// The powers of ten a float is read with: 10^q, for q from {LEAST} to {MOST}, as a
// 128-bit integer times a power of two, its fraction cut off. Each is two
// words, the high one first; the power of two is `(q × 217706) >> 16` less
// 127.
// Written by scripts/float-powers.py; edit that, not this.

// wip fmt: off

/// The least power of ten the table holds.
val LEAST_POWER: i64 = {LEAST}

/// The most.
val MOST_POWER: i64 = {MOST}

/// The powers, from the least.
val POWERS_OF_TEN: [u64; {2 * (MOST - LEAST + 1)}] = [
{chr(10).join(rows)}
]
"""


if __name__ == "__main__":
    path = "std/prelude/float_powers.wip"
    wanted = text()
    if "--check" in sys.argv:
        with open(path, encoding="utf-8") as f:
            if f.read() != wanted:
                sys.exit(f"{path} is not what scripts/float-powers.py writes")
        sys.exit(0)
    with open(path, "w", encoding="utf-8", newline="\n") as f:
        f.write(wanted)
    print(f"wrote {path}: {MOST - LEAST + 1} powers of ten")
