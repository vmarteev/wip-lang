/* Found beside the module's Wip, and compiled with what the module
   defines: `START`, `STEP` and `LOUD` are none of them written here. */
#include <stdint.h>

static int64_t counted = START;

int64_t tally_next(void) {
    counted += STEP;
    return counted;
}

int64_t tally_loud(void) {
#ifdef LOUD
    return LOUD;
#else
    return 0;
#endif
}
