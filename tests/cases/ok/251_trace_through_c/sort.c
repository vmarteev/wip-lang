/* The sort C does, calling back to Wip to compare. */
#include <stdlib.h>

void sort_i64(long long *base, long long count, int (*cmp)(const long long *, const long long *)) {
    qsort(base, (size_t)count, sizeof *base, (int (*)(const void *, const void *))cmp);
}
