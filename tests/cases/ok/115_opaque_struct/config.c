#include <stdlib.h>
#include "config.h"

struct Config *config_new(void) {
    struct Config *c = calloc(1, sizeof *c);
    c->mode = 2;
    c->level = 7;
    c->name = "from C";
    return c;
}

void config_free(struct Config *c) { free(c); }
