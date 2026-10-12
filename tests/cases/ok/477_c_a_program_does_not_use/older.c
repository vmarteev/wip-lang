#include <stdlib.h>
#include "older.h"

struct Kept *kept_new(void) {
    struct Kept *kept = malloc(sizeof *kept);
    kept->flags = 9;
    kept->count = 300;
    return kept;
}

void kept_free(struct Kept *kept) {
    free(kept);
}
