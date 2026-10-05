#include <stdint.h>

int32_t sum4(const int32_t xs[4]) {
    return xs[0] + xs[1] + xs[2] + xs[3];
}

void fill16(uint8_t out[16], uint8_t start) {
    for (int i = 0; i < 16; i++) {
        out[i] = (uint8_t)(start + i);
    }
}

int32_t grid(const int32_t g[2][3]) {
    return g[1][2] * 10 + g[0][1];
}
