/* A tiny C library, so the case needs nothing installed. */
#include <stdlib.h>

struct counter {
    int value;
};

struct counter *counter_open(int start) {
    struct counter *c = malloc(sizeof *c);
    c->value = start;
    return c;
}

int counter_add(struct counter *c, int by) {
    c->value += by;
    return c->value;
}

void counter_close(struct counter *c) {
    free(c);
}

int counter_secret(void) {
    return 7;
}
