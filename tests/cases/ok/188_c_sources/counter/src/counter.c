/* Compiled because the module names it, and configured by the header the
   module puts before it: `START` comes from `build.h`, which this file
   never includes. */
#include <stdint.h>

static int64_t counted = START;

int64_t counter_next(void) {
    return ++counted;
}
