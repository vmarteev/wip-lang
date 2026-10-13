#include "parts.h"

int64_t total(const Part *parts, int64_t count) {
    int64_t sum = 0;
    for (int64_t i = 0; i < count; i++) {
        for (int64_t j = 0; j < parts[i].count; j++) sum += parts[i].values[j];
    }
    return sum;
}

int64_t first_total(const Pair *pair) {
    return total(pair->first, 1);
}
