#include <stdint.h>

typedef struct Span {
    int32_t *values;
    int64_t count;
} Span;

typedef struct Job {
    Span *span;
    uint8_t *label;
    int64_t *total;
    void *tag;
} Job;

int64_t run(const Job *job);
