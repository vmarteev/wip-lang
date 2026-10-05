/* A C library that keeps what it is given and calls back with it later,
   as FSEvents, a timer or an event loop does. */

static void *kept;
static void (*kept_callback)(void *);

void keep(void *data, void (*callback)(void *)) {
    kept = data;
    kept_callback = callback;
}

void fire(int times) {
    for (int i = 0; i < times; i++) {
        kept_callback(kept);
    }
}

void let_go(void) {
    kept = 0;
    kept_callback = 0;
}
