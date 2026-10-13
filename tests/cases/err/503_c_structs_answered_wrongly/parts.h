#include <stdint.h>

typedef struct Part {
    const int32_t *values;
    int64_t count;
} Part;

typedef struct Pair {
    const Part *first;
    const void *tag;
} Pair;

int64_t total(const Part *parts, int64_t count);
int64_t first_total(const Pair *pair);
