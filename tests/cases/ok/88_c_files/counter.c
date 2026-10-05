/* A `.c` file of a Wip module: compiled with `cc` and linked with the
   program, so a case can talk to C with nothing installed. */
#include "counter.h"

static long long counter = 0;

long long counter_next(void) {
    counter += 1;
    return counter;
}

long long counter_add(long long a, long long b) {
    return a + b;
}
