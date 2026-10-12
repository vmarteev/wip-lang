#include <string.h>

#include "lent.h"

static int thing;

void listed(uint32_t *count, uint32_t *items) {
    if (!items) {
        *count = 3;
        return;
    }
    for (uint32_t i = 0; i < *count; i++) {
        items[i] = 10 + i;
    }
}

void made(const int32_t *info, Thing **out) {
    *out = *info == 7 ? (Thing *)&thing : 0;
}

int64_t summed(const uint32_t *items, int64_t count) {
    int64_t sum = 0;
    for (int64_t i = 0; i < count; i++) {
        sum += items[i];
    }
    return sum;
}

void cleared(void *at) {
    memset(at, 0, sizeof(int64_t));
}
