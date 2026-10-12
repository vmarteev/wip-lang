#include <stdint.h>

static int32_t kept[4] = {1, 2, 3, 4};

int32_t *numbers(void) {
    return kept;
}

int32_t summed(int32_t count) {
    int32_t sum = 0;
    for (int32_t i = 0; i < count; i++) {
        sum += kept[i];
    }
    return sum;
}
