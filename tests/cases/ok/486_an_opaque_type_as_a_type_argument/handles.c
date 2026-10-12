#include <stdint.h>
#include <stdlib.h>

typedef struct Handle {
    int64_t n;
} Handle;

Handle *handle_new(int64_t n) {
    static Handle handles[4];
    handles[n].n = n;
    return &handles[n];
}

int64_t handle_number(Handle *h) {
    return h->n;
}
