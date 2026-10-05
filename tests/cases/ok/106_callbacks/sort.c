/* C calls Wip, both ways round. The sort asks for a
   comparator, and `scale_all` calls a Wip function by its own name. */
#include <stdlib.h>

void sort_i64(long long *base, long long count, int (*cmp)(const long long *, const long long *)) {
    qsort(base, (size_t)count, sizeof *base, (int (*)(const void *, const void *))cmp);
}

extern long long wip_scale(long long value);

long long scale_all(long long *base, long long count) {
    long long total = 0;
    for (long long i = 0; i < count; i += 1) {
        base[i] = wip_scale(base[i]);
        total += base[i];
    }
    return total;
}
