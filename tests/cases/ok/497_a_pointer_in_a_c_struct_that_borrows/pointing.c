#include "pointing.h"

int64_t run(const Job *job) {
    int64_t sum = 0;
    for (int64_t i = 0; i < job->span->count; i++) {
        sum += job->span->values[i];
    }
    *job->total = sum;
    return sum + job->label[0] + *(const int64_t *)job->tag;
}
