/* Arguments narrower than 32 bits, and `float`, must arrive intact from
   Wip code on every platform: Apple's arm64 ABI has the
   caller extend them, and a callee compiled by `cc` relies on it. */
#include <stdint.h>

int64_t wip_sum_args(uint8_t a, int16_t b, uint32_t c, float d) {
    return (int64_t)a + b + c + (int64_t)(d * 4);
}
